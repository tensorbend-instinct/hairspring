//! RED acceptance gates for Phase 4 (D7 evolution loop, AVO pattern).
//! Behavioral: a candidate PROMPT is benched through real missions against
//! the parent; promotion happens only when the candidate BEATS the parent
//! on held-out tasks; rejections record reasons; rewind restores the parent.
//!
//! The gatemodel fixture makes prompt content load-bearing: it only writes
//! the secret when the mission prompt carries the marker line, so a prompt
//! candidate's worth is measurable through real mission outcomes.

use hs_loop::sweprompt::PromptArgs;
use hs_loop::*;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const GATEMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-gatemodel");

/// gatemodel writes TOKEN-0-SECRET only when the prompt carries this marker
const MARKER: &str = "EVO-PREFLIGHT-LAW";

fn rig(dir: &std::path::Path, log: &std::path::Path, max_steps: u32) -> InnerLoop {
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

fn task_dirs(base: &std::path::Path, name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let d = base.join(name);
    std::fs::create_dir_all(&d).unwrap();
    (d.clone(), d.join("log"))
}

#[test]
fn d7_promote_only_verified_winners_reject_losers_rewind_restores() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let base = tempfile::tempdir().unwrap();
    let overlay = base.path().join("policy.toml");
    let journal = base.path().join("evolution.jsonl");

    // one real mission per (prompt, task): the prompt is built from the
    // template under test; the gatemodel makes the marker load-bearing
    let runner = |template: Option<&str>, task: &str| -> evolve::BenchOutcome {
        let (dir, log) = task_dirs(base.path(), &format!("{task}-{}", uuid::Uuid::new_v4()));
        let mut l = rig(&dir, &log, 3);
        let stream = l.stream_id();
        let prompt = if let Some(t) = template { t.replace(
            "{answer_path}",
            &log.join("work")
                .join(task)
                .join("answer.txt")
                .display()
                .to_string(),
        ) } else {
            let args = PromptArgs {
                ws: ".".into(),
                problem_statement: "p".into(),
                fail_to_pass: vec![],
                repo_layout: String::new(),
                nudge: String::new(),
                answer_path: log
                    .join("work")
                    .join(task)
                    .join("answer.txt")
                    .display()
                    .to_string(),
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
    };

    let parent: Option<String> = None; // builtin template (no marker)
    let winner = format!("{MARKER}\nYou are fixing a bug. {{{{answer_path}}}} is your ANSWER_PATH.\nANSWER_PATH: {{answer_path}}\nATTEMPT: 1");
    let loser = "no marker here.\nANSWER_PATH: {answer_path}\nATTEMPT: 1".to_string();

    let bench = vec!["task-0".to_string(), "task-1".to_string()];
    let held_out = vec!["task-2".to_string()];

    // 1. loser: ties the parent (both fail) -> REJECTED with a reason
    let d1 = evolve::evaluate_candidate(
        &runner,
        parent.clone(),
        loser.clone(),
        &bench,
        &held_out,
        &overlay,
        &journal,
    );
    assert!(
        matches!(d1.decision, evolve::Decision::Rejected(_)),
        "loser rejected: {d1:?}"
    );
    assert!(!overlay.exists(), "rejection never touches the overlay");
    let j = std::fs::read_to_string(&journal).unwrap();
    assert!(
        j.contains("rejected") && j.contains("reason"),
        "rejection recorded with reason: {j}"
    );

    // 2. winner: passes everywhere the parent fails -> PROMOTED, lineage booked
    let d2 = evolve::evaluate_candidate(
        &runner,
        parent.clone(),
        winner.clone(),
        &bench,
        &held_out,
        &overlay,
        &journal,
    );
    assert!(
        matches!(d2.decision, evolve::Decision::Promoted),
        "winner promoted: {d2:?}"
    );
    let ov = std::fs::read_to_string(&overlay).unwrap();
    assert!(
        ov.contains(MARKER),
        "overlay now carries the candidate: {ov}"
    );
    let j = std::fs::read_to_string(&journal).unwrap();
    assert!(
        j.contains("promoted") && j.contains("parent_hash"),
        "lineage refs the parent: {j}"
    );
    // chain-verified traces: the journal names the bench mission streams
    for b in d2.bench.iter().chain(d2.held_out_candidate.iter()) {
        assert!(
            j.contains(&b.stream_id.to_string()),
            "journal refs trace {}",
            b.stream_id
        );
    }

    // 3. rewind: the parent template is restored (exo rollback)
    evolve::rewind(&journal, &overlay).unwrap();
    assert!(
        !overlay.exists(),
        "rewind to a builtin parent removes the overlay"
    );
    let j = std::fs::read_to_string(&journal).unwrap();
    assert!(j.contains("rewound"), "rewind recorded: {j}");
    // and behavior reverts: the marker is gone, gatemodel fails again
    let r = runner(None, "task-3");
    assert!(!r.passed, "post-rewind behavior is the parent's");
}
