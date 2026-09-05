//! Phase 1 RED (design D4 mission half + D6 budgets): when a plugin dies
//! for good, the mission ABORTS as harness_error with partial progress
//! booked (never burns remaining steps against a dead plugin, never crashes
//! the runner); and the loop checkpoints progress.json after every step so
//! an external wall-clock kill can book steps-so-far instead of a 0-step row.

use hs_loop::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const BENCHMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-benchmodel");
const SEQMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");
const FIXTURE: &str = env!("CARGO_BIN_EXE_hs-loopfix");

static SEQMODEL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn write(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p
}

/// T3 (mission half, REVISED by measurement run ab2/17123): death of an
/// ANSWER-PATH plugin (answer.write, edit.apply) aborts the mission as
/// harness_error with steps-so-far - without an answer path no mission can
/// land, so burning steps is pure loss (the old 17117 lesson). Death of any
/// OTHER tool degrades instead of aborting: see T3b/T3c.
#[test]
fn mission_aborts_as_harness_error_when_answer_path_plugin_dies() {
    let _g = SEQMODEL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let answer = log.path().join("work").join("task-0").join("answer.txt");
    let script = write(
        dir.path(),
        "script.jsonl",
        &format!(
            r#"{{"tool":"answer.write","args":{{"path":"{}","content":"TOKEN-0-SECRET"}}}}"#,
            answer.display()
        ),
    );
    std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
    let config = write(
        dir.path(),
        "hairspring.toml",
        &format!(
            r#"
[[tools]]
name = "answer.write"
command = ["{FIXTURE}", "answer.write"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{CHECKER}"]
subjects = ["*"]

[[models]]
name = "scripted"
command = ["{SEQMODEL}"]
default = true
"#
        ),
    );
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 10).unwrap();
    let r = l.run_mission("task-0").unwrap();
    assert_eq!(r.passed, false);
    assert_eq!(r.steps, 1, "abort on the failing step, not the cap: {:?}", r.steps);
    assert!(
        r.harness_error.is_some(),
        "answer-path death must abort as harness_error, got: {r:?}"
    );
    let msg = r.harness_error.unwrap();
    assert!(msg.contains("answer.write"), "names the dead plugin: {msg}");
    let reader = hs_log::StreamReader::open(log.path(), r.stream_id).unwrap();
    let events = reader.events().unwrap();
    let booked = events.iter().any(|e| {
        e.kind == hs_core::EventKind::Feedback && {
            let b = reader.resolve_payload(e).unwrap_or_default();
            let s = String::from_utf8_lossy(&b);
            s.contains("harness_error") && s.contains("answer.write")
        }
    });
    assert!(booked, "harness_error abort must be booked as a Feedback event");
}

/// T3b (ab2/17123): death of a NON-answer tool degrades gracefully - the
/// mission continues with the remaining tools and can still PASS. repo.exec
/// dying must not kill a mission whose answer path is intact: 17123 aborted
/// at 13 steps with edit.apply/answer.write fully usable.
#[test]
fn dead_non_answer_tool_degrades_and_mission_continues() {
    let _g = SEQMODEL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let answer = log.path().join("work").join("task-0").join("answer.txt");
    let script = write(
        dir.path(),
        "script.jsonl",
        &format!(
            "{{\"tool\":\"zombie\",\"args\":{{}}}}\n{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-0-SECRET\"}}}}",
            answer.display()
        ),
    );
    std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
    let config = write(
        dir.path(),
        "hairspring.toml",
        &format!(
            r#"
[[tools]]
name = "answer.write"
command = ["{ANSWER}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{CHECKER}"]
subjects = ["*"]

[[tools]]
name = "zombie"
command = ["{FIXTURE}"]
subjects = ["*"]

[[models]]
name = "scripted"
command = ["{SEQMODEL}"]
default = true
"#
        ),
    );
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 10).unwrap();
    let r = l.run_mission("task-0").unwrap();
    assert!(
        r.passed,
        "mission must still pass via the intact answer path: {r:?}"
    );
    assert_eq!(r.steps, 2, "zombie dies step 1, answer lands step 2");
    assert!(
        r.harness_error.is_none(),
        "non-answer death must NOT abort: {:?}",
        r.harness_error
    );
    // the degradation is booked trace-visibly and names the dead tool
    let reader = hs_log::StreamReader::open(log.path(), r.stream_id).unwrap();
    let events = reader.events().unwrap();
    let booked = events.iter().any(|e| {
        let b = reader.resolve_payload(e).unwrap_or_default();
        let s = String::from_utf8_lossy(&b);
        s.contains("zombie") && s.contains("unavailable")
    });
    assert!(booked, "degradation must be booked naming the dead tool");
}

/// T3c: repeat calls to a dead tool short-circuit at the loop with
/// feedback - the kernel is never asked to respawn it (loopfix state file
/// counts real spawns: exactly MAX_STRIKES, no more).
#[test]
fn repeat_calls_to_dead_tool_short_circuit_without_respawn() {
    let _g = SEQMODEL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let state = dir.path().join("zombie.state");
    let answer = log.path().join("work").join("task-0").join("answer.txt");
    let script = write(
        dir.path(),
        "script.jsonl",
        &format!(
            "{{\"tool\":\"zombie\",\"args\":{{}}}}\n{{\"tool\":\"zombie\",\"args\":{{}}}}\n{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-0-SECRET\"}}}}",
            answer.display()
        ),
    );
    std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
    let config = write(
        dir.path(),
        "hairspring.toml",
        &format!(
            r#"
[[tools]]
name = "answer.write"
command = ["{ANSWER}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{CHECKER}"]
subjects = ["*"]

[[tools]]
name = "zombie"
command = ["{FIXTURE}", "zombie", "{}"]
subjects = ["*"]

[[models]]
name = "scripted"
command = ["{SEQMODEL}"]
default = true
"#,
            state.display()
        ),
    );
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 10).unwrap();
    let r = l.run_mission("task-0").unwrap();
    assert!(r.passed, "mission recovers after the dead-tool detour: {r:?}");
    assert_eq!(r.steps, 3);
    let spawns = std::fs::read_to_string(&state).unwrap().lines().count();
    assert_eq!(
        spawns, 3,
        "only the original MAX_STRIKES spawn attempts, never a respawn: {spawns}"
    );
}

/// T5a: the loop checkpoints progress.json after EVERY step (external
/// wall-clock kill must never produce a 0-step result for a run that worked).
#[test]
fn progress_json_checkpointed_per_step() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let progress = log.path().join("progress.json");
    let config = write(
        dir.path(),
        "hairspring.toml",
        &format!(
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
name = "benchmodel"
command = ["{BENCHMODEL}"]
default = true
"#
        ),
    );
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 3).unwrap();
    l.set_progress_path(&progress);
    // task-18 is unrepairable: runs to the cap without passing
    let r = l.run_mission("task-18").unwrap();
    assert!(!r.passed);
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&progress).unwrap()).unwrap();
    assert_eq!(v["steps"].as_u64().unwrap(), 3, "checkpoint tracks steps");
    assert_eq!(v["model_calls"].as_u64().unwrap(), 3);
    assert_eq!(r.steps, 3);
}

/// T5c (measurement run ab2, missions 17092/17102/17117): the checkpoint
/// must land after EVERY step - including steps with no answer.write. All
/// three wall-killed ab2 missions left NO progress.json because only
/// answer-steps and abort paths checkpointed, so the runner booked 0-step
/// rows for runs that did 19-25 real steps.
#[test]
fn progress_checkpointed_every_step_even_without_answer_writes() {
    let _g = SEQMODEL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let progress = log.path().join("progress.json");
    let script = write(dir.path(), "script.jsonl", "not a json tool call at all");
    std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
    let config = write(
        dir.path(),
        "hairspring.toml",
        &format!(
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
name = "scripted"
command = ["{SEQMODEL}"]
default = true
"#
        ),
    );
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 3).unwrap();
    l.set_progress_path(&progress);
    // malformed model output every step: no tool call, no answer - the loop
    // still did 3 real model calls that a wall kill must book
    let r = l.run_mission("task-0").unwrap();
    assert!(!r.passed);
    assert_eq!(r.steps, 3);
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&progress).unwrap()).unwrap();
    assert_eq!(v["steps"].as_u64().unwrap(), 3, "every step checkpoints, answer or not");
    assert_eq!(v["model_calls"].as_u64().unwrap(), 3);
}

/// T5b: booking a wall kill from the checkpoint yields the true step count,
/// never the 0-step row the old runner wrote on timeout.
#[test]
fn wall_kill_books_partial_from_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let progress = dir.path().join("progress.json");
    std::fs::write(
        &progress,
        r#"{"steps": 22, "model_calls": 22, "cost_micros": 12345}"#,
    )
    .unwrap();
    let result = book_wall_kill(&progress, "conan-io__conan-17102", "kimi", true);
    assert_eq!(result["passed"], false);
    assert_eq!(result["steps"], 22, "true steps, not 0");
    assert_eq!(result["model_calls"], 22);
    assert_eq!(result["outcome"], "wall_killed");
    assert_eq!(result["instance_id"], "conan-io__conan-17102");
    assert_eq!(result["cost_micros"], 12345);
}

/// Fix 4 (Eric 2026-09-05, ab2 forensics): the volatile header must carry
/// the budgets every step - "step N of MAX, T-minus Xs, $Y of $Z spent" -
/// so the model can pace itself. ab2 proved the old bare "ATTEMPT: N" left
/// the model blind: it read its way into the wall.
#[test]
fn step_header_carries_step_wall_and_cost_budgets() {
    let _g = SEQMODEL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let answer = log.path().join("work").join("task-b").join("answer.txt");
    let script = write(
        dir.path(),
        "script.jsonl",
        &format!(
            "{{\"tool\":\"bogus.noop\",\"args\":{{}}}}\n{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-b-SECRET\"}}}}",
            answer.display()
        ),
    );
    std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
    let config = write(
        dir.path(),
        "hairspring.toml",
        &format!(
            r#"
[[tools]]
name = "answer.write"
command = ["{FIXTURE}", "answer.write"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{CHECKER}"]
subjects = ["*"]

[[models]]
name = "scripted"
command = ["{SEQMODEL}"]
default = true
"#
        ),
    );
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 3).unwrap();
    l.set_wall_secs(3600);
    l.set_budget_micros(10_000_000);
    let r = l.run_mission("task-b").unwrap();
    let reader = hs_log::StreamReader::open(log.path(), r.stream_id).unwrap();
    let prompts: Vec<String> = reader
        .events()
        .unwrap()
        .iter()
        .filter(|e| e.kind == hs_core::EventKind::ModelCall)
        .map(|e| {
            let b = reader.resolve_payload(e).unwrap();
            let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
            v["prompt"].as_str().unwrap_or("").to_string()
        })
        .collect();
    assert_eq!(prompts.len(), 2, "scripted mission ran 2 steps: {r:?}");
    let p2 = &prompts[1];
    assert!(p2.contains("step 2 of 3"), "step N of MAX: {}", &p2[..p2.len().min(400)]);
    assert!(p2.contains("T-minus"), "wall remaining: {}", &p2[..p2.len().min(400)]);
    assert!(p2.contains("of $10.00 spent"), "cost vs budget: {}", &p2[..p2.len().min(400)]);
}

/// Fix 5 (ab2: three wall-killed missions ran 19-25 steps with ZERO
/// model-initiated verification - the model read its way into the wall).
/// At 50% and 75% of the step budget, when the model has never run a test
/// or check itself, the volatile header must carry a CONVERGENCE nudge and
/// the resident LEDGER must flag "NO TEST RUN YET". The harness's own
/// checker.run verdicts do NOT count: they are ground truth, not the model
/// verifying its work.
#[test]
fn convergence_nudge_at_half_and_three_quarter_steps_when_no_test_ran() {
    let _g = SEQMODEL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let answer = log.path().join("work").join("task-20").join("answer.txt");
    // step 1 writes a WRONG answer (checker fails -> mission continues);
    // steps 2-4 fumble with an unknown tool. The model never runs a test.
    let script = write(
        dir.path(),
        "script.jsonl",
        &format!(
            "{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"WRONG\"}}}}\n{{\"tool\":\"bogus.noop\",\"args\":{{}}}}\n{{\"tool\":\"bogus.noop\",\"args\":{{}}}}\n{{\"tool\":\"bogus.noop\",\"args\":{{}}}}",
            answer.display()
        ),
    );
    std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
    let config = write(
        dir.path(),
        "hairspring.toml",
        &format!(
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
name = "scripted"
command = ["{SEQMODEL}"]
default = true
"#
        ),
    );
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 4).unwrap();
    let r = l.run_mission("task-20").unwrap();
    assert!(!r.passed, "a wrong answer the model never re-verifies cannot pass: {r:?}");
    let reader = hs_log::StreamReader::open(log.path(), r.stream_id).unwrap();
    let prompts: Vec<String> = reader
        .events()
        .unwrap()
        .iter()
        .filter(|e| e.kind == hs_core::EventKind::ModelCall)
        .map(|e| {
            let b = reader.resolve_payload(e).unwrap();
            let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
            v["prompt"].as_str().unwrap_or("").to_string()
        })
        .collect();
    assert_eq!(prompts.len(), 4, "mission ran to the 4-step cap: {r:?}");
    assert!(
        !prompts[0].contains("CONVERGENCE:"),
        "no nudge before the halfway mark: {}",
        &prompts[0][..prompts[0].len().min(400)]
    );
    for (i, label) in [(1usize, "halfway (step 2 of 4)"), (2usize, "three-quarter (step 3 of 4)")] {
        assert!(
            prompts[i].contains("CONVERGENCE:"),
            "convergence nudge at {label}: {}",
            &prompts[i][..prompts[i].len().min(500)]
        );
    }
    assert!(
        prompts[1].contains("NO TEST RUN YET"),
        "ledger flags missing verification once work exists: {}",
        &prompts[1][..prompts[1].len().min(900)]
    );
}
