//! RED acceptance (Eric 2026-09-13): the prompt-level RSI loop, wired live.
//! The machinery was proven in-crate and dark in every live path (RSI
//! evidence hunt, 2026-09-13): hs-plugin-policy uninstalled, nothing
//! stamped a mission run dir, the [prompts] overlay loaded only through an
//! env var nothing set, and evolve::evaluate_candidate had no production
//! caller. These gates pin the LIVE wiring:
//!   k1: the kernel stamps the mission run dir on every tool.call;
//!       policy.propose_prompt records into it with NO env var.
//!   k2: the TUI mission prompt is policy: default passthrough (goal
//!       verbatim), a [prompts] tui-mission overlay wraps it.
//!   k3: the promotion driver: latest proposal -> fixture bench -> winner
//!       promotes into the overlay + journal; a loser is rejected with a
//!       reason; rewind restores the builtin parent.
//!   k4: live overlay resolution: HS_POLICY_TOML wins, else the canonical
//!       config-dir overlay when it exists, else builtin.
//!   k5: a scripted REPL mission that calls policy.propose_prompt records
//!       the proposal under <log>/work/<mission>/ - the live session path.

use hs_loop::repl::load_session;
use hs_loop::sweprompt;
use hs_loop::*;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const GATEMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-gatemodel");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");
const POLICY: &str = env!("CARGO_BIN_EXE_hs-plugin-policy");

/// gatemodel writes TOKEN-0-SECRET only when the prompt carries this marker
const MARKER: &str = "EVO-PREFLIGHT-LAW";

fn policy_config(dir: &std::path::Path) -> std::path::PathBuf {
    let config = dir.join("hairspring.toml");
    std::fs::write(
        &config,
        format!(
            r#"
[[tools]]
name = "policy.propose_prompt"
command = ["{POLICY}"]
subjects = ["*"]

[[models]]
name = "gatemodel"
command = ["{GATEMODEL}"]
default = true
"#
        ),
    )
    .unwrap();
    config
}

#[test]
fn k1_kernel_stamps_run_dir_and_policy_plugin_records() {
    let _g = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    unsafe { std::env::remove_var("HS_RUN_DIR") };
    let dir = tempfile::tempdir().unwrap();
    let kernel = hs_kernel::Kernel::load(&policy_config(dir.path())).unwrap();
    // No run_dir, no env: the error must be actionable, never silent.
    let err = kernel
        .call_tool(
            "operator",
            "policy.propose_prompt",
            serde_json::json!({"name": "swe-mission", "text": "X"}),
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("run_dir"),
        "error must name the missing run_dir: {err}"
    );
    // What the loop does at mission start: stamp the mission's dir.
    let mission_dir = dir.path().join("log/work/mission-x");
    std::fs::create_dir_all(&mission_dir).unwrap();
    kernel.set_tool_run_dir(Some(mission_dir.clone()));
    let out = kernel
        .call_tool(
            "operator",
            "policy.propose_prompt",
            serde_json::json!({"name": "swe-mission", "text": "always read first"}),
        )
        .unwrap();
    assert_eq!(out.output["recorded"], serde_json::json!(true), "{:?}", out.output);
    let log = std::fs::read_to_string(mission_dir.join("policy_proposals.jsonl")).unwrap();
    let rec: serde_json::Value = serde_json::from_str(log.lines().next().unwrap()).unwrap();
    assert_eq!(rec["version"], 1);
    assert_eq!(rec["parent_hash"], "genesis");
    assert_eq!(rec["status"], "proposed");
    assert_eq!(rec["text"], "always read first");
}

#[test]
fn k2_tui_mission_prompt_is_policy() {
    let dir = tempfile::tempdir().unwrap();
    // Default passthrough: the goal IS the prompt (dance #95), MCP block
    // behavior byte-equal to the pre-policy path.
    assert_eq!(
        sweprompt::build_tui_mission_prompt(None, "fix the bug", ""),
        "fix the bug"
    );
    assert_eq!(
        sweprompt::build_tui_mission_prompt(None, "fix the bug", "mcp.one: does x"),
        "fix the bug\n\nAVAILABLE MCP TOOLS (call them like any other tool):\nmcp.one: does x"
    );
    let overlay_path = dir.path().join("policy.toml");
    std::fs::write(
        &overlay_path,
        "[prompts]\ntui-mission = \"STANDING LAW\\n{goal}\\nEND LAW\"\n",
    )
    .unwrap();
    let overlay = sweprompt::load_policy_overlay(&overlay_path).unwrap();
    assert_eq!(
        sweprompt::build_tui_mission_prompt(Some(&overlay), "fix the bug", ""),
        "STANDING LAW\nfix the bug\nEND LAW"
    );
}

fn fixture_rig(dir: &std::path::Path, log: &std::path::Path, max_steps: u32) -> InnerLoop {
    let config = dir.join("hairspring.toml");
    std::fs::write(
        &config,
        format!(
            r#"
[[tools]]
name = "answer.write"
command = ["{ANSWER}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{CHECKER}"]
subjects = ["*"]

[[models]]
name = "gatemodel"
command = ["{GATEMODEL}"]
default = true
"#
        ),
    )
    .unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    InnerLoop::new(kernel, log, true, max_steps).unwrap()
}

/// One real mission per (prompt, task): the gatemodel makes prompt content
/// load-bearing, so a candidate's worth is measurable through outcomes.
fn fixture_runner(base: &std::path::Path) -> impl Fn(Option<&str>, &str) -> evolve::BenchOutcome + '_ {
    move |template, task| {
        let d = base.join(format!("{task}-{}", uuid::Uuid::new_v4()));
        let log = d.join("log");
        std::fs::create_dir_all(&log).unwrap();
        let mut l = fixture_rig(&d, &log, 3);
        let stream = l.stream_id();
        let answer = log.join("work").join(task).join("answer.txt");
        let prompt = if let Some(t) = template {
            t.replace("{answer_path}", &answer.display().to_string())
        } else {
            let args = sweprompt::PromptArgs {
                ws: ".".into(),
                problem_statement: "p".into(),
                fail_to_pass: vec![],
                repo_layout: String::new(),
                nudge: String::new(),
                answer_path: answer.display().to_string(),
                orientation: String::new(),
                mcp_tools: String::new(),
            };
            sweprompt::build_mission_prompt(None, &args)
        };
        let r = l.run_mission_full(task, &prompt).unwrap();
        evolve::BenchOutcome {
            task: task.into(),
            passed: r.passed,
            steps: r.steps,
            cost_micros: 0,
            stream_id: stream,
        }
    }
}

#[test]
fn k3_promotion_driver_proposal_to_overlay_journal_rewind() {
    let _g = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    // A proposals log as a mission records it: versioned, hash-chained.
    let pdir = dir.path().join("mission-run");
    std::fs::create_dir_all(&pdir).unwrap();
    let proposals = pdir.join("policy_proposals.jsonl");
    let loser_text = "no marker here.\nANSWER_PATH: {answer_path}\nATTEMPT: 1".to_string();
    let winner_text = format!("{MARKER}\nANSWER_PATH: {{answer_path}}\nATTEMPT: 1");
    let p1 = sweprompt::propose_prompt(&pdir, "swe-mission", &loser_text).unwrap();
    let p2 = sweprompt::propose_prompt(&pdir, "swe-mission", &winner_text).unwrap();
    assert_eq!(p2.parent_hash, p1.hash, "hash chain");
    // The driver benches the LATEST proposal as the candidate.
    let cand = promote::latest_proposal(&proposals).unwrap();
    assert_eq!(cand.version, 2);
    assert_eq!(cand.hash, p2.hash);

    let overlay = dir.path().join("policy.toml");
    let journal = dir.path().join("policy_journal.jsonl");
    let bench_f = dir.path().join("bench.txt");
    let held_f = dir.path().join("held.txt");
    std::fs::write(&bench_f, "task-0\ntask-1\n").unwrap();
    std::fs::write(&held_f, "task-2\n").unwrap();
    let bench = promote::load_task_list(&bench_f).unwrap();
    let held = promote::load_task_list(&held_f).unwrap();

    // No overlay yet: the parent is the builtin template.
    assert_eq!(promote::parent_template(&overlay, "swe-mission").unwrap(), None);

    let runner = fixture_runner(dir.path());
    let eval = evolve::evaluate_candidate_named(
        &runner,
        None,
        cand.text.clone(),
        &bench,
        &held,
        &overlay,
        &journal,
        "swe-mission",
    );
    assert_eq!(eval.decision, evolve::Decision::Promoted);

    // Promotion wrote the overlay entry; sibling entries survive (k3b).
    let promoted = promote::parent_template(&overlay, "swe-mission").unwrap();
    assert_eq!(promoted.as_deref(), Some(winner_text.as_str()));

    // Journal: promoted, with lineage back to the builtin parent.
    let j = std::fs::read_to_string(&journal).unwrap();
    let rec: serde_json::Value = serde_json::from_str(j.lines().next().unwrap()).unwrap();
    assert_eq!(rec["decision"], "promoted");
    assert_eq!(rec["name"], "swe-mission");
    assert_eq!(rec["parent_hash"], "builtin");
    // The journal tracks the TEMPLATE hash (evolve lineage), distinct from
    // the proposal record hash (provenance chain).
    assert_eq!(rec["candidate_hash"], sweprompt::content_hash(&cand.text));

    // A loser against the promoted parent is rejected with a reason and
    // the overlay is untouched.
    let eval2 = evolve::evaluate_candidate_named(
        &runner,
        Some(winner_text.clone()),
        loser_text.clone(),
        &bench,
        &held,
        &overlay,
        &journal,
        "swe-mission",
    );
    assert!(
        matches!(eval2.decision, evolve::Decision::Rejected(_)),
        "{:?}",
        eval2.decision
    );
    assert_eq!(
        promote::parent_template(&overlay, "swe-mission").unwrap().as_deref(),
        Some(winner_text.as_str())
    );

    // Rewind restores the builtin parent: the entry drops, the file goes.
    evolve::rewind_named(&journal, &overlay, "swe-mission").unwrap();
    assert!(!overlay.exists(), "rewind to builtin removes the overlay");
}

#[test]
fn k3b_promotion_preserves_sibling_prompt_entries() {
    let _g = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let overlay = dir.path().join("policy.toml");
    std::fs::write(
        &overlay,
        "[prompts]\ntui-mission = \"TUI LAW {goal}\"\nswe-mission = \"old swe\"\n",
    )
    .unwrap();
    let journal = dir.path().join("policy_journal.jsonl");
    let runner = fixture_runner(dir.path());
    let winner = format!("{MARKER}\nANSWER_PATH: {{answer_path}}\nATTEMPT: 1");
    let bench = vec!["task-0".to_string()];
    let held = vec!["task-1".to_string()];
    let parent = promote::parent_template(&overlay, "swe-mission").unwrap();
    let eval = evolve::evaluate_candidate_named(
        &runner,
        parent,
        winner.clone(),
        &bench,
        &held,
        &overlay,
        &journal,
        "swe-mission",
    );
    assert_eq!(eval.decision, evolve::Decision::Promoted);
    // tui-mission untouched by an swe-mission promotion.
    assert_eq!(
        promote::parent_template(&overlay, "tui-mission").unwrap().as_deref(),
        Some("TUI LAW {goal}")
    );
    // Rewind restores the previous swe-mission entry, tui-mission intact.
    evolve::rewind_named(&journal, &overlay, "swe-mission").unwrap();
    assert_eq!(
        promote::parent_template(&overlay, "swe-mission").unwrap().as_deref(),
        Some("old swe")
    );
    assert_eq!(
        promote::parent_template(&overlay, "tui-mission").unwrap().as_deref(),
        Some("TUI LAW {goal}")
    );
}

#[test]
fn k4_live_overlay_resolution() {
    let _g = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", dir.path());
        std::env::remove_var("HS_POLICY_TOML");
    }
    // No canonical overlay: builtin.
    assert_eq!(sweprompt::resolve_policy_overlay_path(), None);
    // Canonical overlay exists: it wins by default.
    let hs = dir.path().join("hairspring");
    std::fs::create_dir_all(&hs).unwrap();
    let canon = hs.join("policy.toml");
    std::fs::write(&canon, "[prompts]\n").unwrap();
    assert_eq!(sweprompt::resolve_policy_overlay_path(), Some(canon));
    // Explicit env beats the canonical promoted overlay.
    unsafe { std::env::set_var("HS_POLICY_TOML", "/tmp/explicit-policy.toml") };
    assert_eq!(
        sweprompt::resolve_policy_overlay_path(),
        Some(std::path::PathBuf::from("/tmp/explicit-policy.toml"))
    );
    unsafe {
        std::env::remove_var("HS_POLICY_TOML");
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[test]
fn k5_repl_mission_records_proposal_under_mission_dir() {
    let _g = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    let script = dir.path().join("script.jsonl");
    std::fs::write(
        &script,
        concat!(
            "{\"tool\":\"policy.propose_prompt\",\"args\":{\"name\":\"tui-mission\",\"text\":\"EVO-PREFLIGHT-LAW {goal}\"}}\n",
            "{\"tool\":\"answer.write\",\"args\":{\"path\":\"ANSWER\",\"content\":\"done\"}}\n"
        )
        .replace("ANSWER", &log.join("work/m1/answer.txt").display().to_string()),
    )
    .unwrap();
    unsafe {
        std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
        std::env::remove_var("HS_RUN_DIR");
        std::env::remove_var("HS_POLICY_TOML");
        // No overlay anywhere: the session runs builtin passthrough.
        std::env::set_var("XDG_CONFIG_HOME", dir.path().join("xdg"));
    }
    let config = dir.path().join("hairspring.toml");
    std::fs::write(
        &config,
        format!(
            r#"
[[tools]]
name = "policy.propose_prompt"
command = ["{POLICY}"]
subjects = ["*"]

[[tools]]
name = "answer.write"
command = ["{ANSWER}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{CHECKER}"]
subjects = ["*"]

[[models]]
name = "scripted"
command = ["{SCRIPTED}"]
default = true
subjects = ["*"]
"#
        ),
    )
    .unwrap();
    let mut session = load_session(&config, &log, true, Some(6), None, None).unwrap();
    let _ = session.run_goal("write hello.txt containing hello");
    // The proposal landed under the mission's own dir, whichever slug the
    // goal mapped to.
    let mut found = vec![];
    for e in std::fs::read_dir(log.join("work")).unwrap() {
        let p = e.unwrap().path().join("policy_proposals.jsonl");
        if p.exists() {
            found.push(p);
        }
    }
    assert_eq!(found.len(), 1, "exactly one mission dir holds proposals: {found:?}");
    let text = std::fs::read_to_string(&found[0]).unwrap();
    let rec: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(rec["name"], "tui-mission");
    assert!(rec["text"].as_str().unwrap().contains(MARKER));
    assert_eq!(rec["status"], "proposed");
}
