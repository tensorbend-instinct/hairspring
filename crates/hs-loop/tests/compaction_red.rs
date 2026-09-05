//! GATE 9f (spec v5, "When to compact"): window-pressure compaction.
//!
//! "When to compact. On window pressure, not on a fixed schedule."
//! "if window_pressure(ctx) > HIGH_WATER: ctx = compact(ctx);
//!  record(context_inject, why=pressure)"
//! "Older segments distill into compaction summaries that link back to
//!  source event ranges, so distillation never destroys auditability."
//!
//! Falsifiable: (1) a mission whose transcript overflows HIGH_WATER must
//! carry a COMPACTED summary naming the source event range in a later
//! prompt, and the stream must record a context_inject event with
//! why=pressure; (2) a mission under the watermark compacts NOTHING - no
//! summary, no context_inject. Today's fixed 60KB cap silently DROPS the
//! oldest lines: no summary, no link back, no event - that is the red.

use hs_core::{EventKind, Payload};
use hs_loop::*;

// REC_DUMP/REC_MODE are process-global env vars consumed by the recmodel
// plugin child process: the two scenarios must not run concurrently.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const PROBE: &str = env!("CARGO_BIN_EXE_hs-plugin-probe");
const BIGREAD: &str = env!("CARGO_BIN_EXE_hs-plugin-bigread");
const RECMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-recmodel");

fn rig(dir: &std::path::Path, log: &std::path::Path) -> InnerLoop {
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

[[tools]]
name = "probe.read"
command = ["{PROBE}"]
subjects = ["*"]

[[tools]]
name = "bigread.read"
command = ["{BIGREAD}"]
subjects = ["*"]

[[models]]
name = "recmodel"
command = ["{RECMODEL}"]
default = true
"#
        ),
    )
    .unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log, true, 8).unwrap();
    // the D1 budget is token-sized and configurable; this rig uses a small
    // one so the pressure scenario still exercises the compression path
    l.set_context_budget_tokens(16_000);
    l
}

fn context_injects(log: &std::path::Path, stream: uuid::Uuid) -> Vec<String> {
    let reader = hs_log::StreamReader::open(log, stream).unwrap();
    reader
        .events()
        .unwrap()
        .iter()
        .filter(|e| e.kind == EventKind::ContextInject)
        .filter_map(|e| match &e.payload {
            Payload::Inline(b) => Some(String::from_utf8_lossy(b).into_owned()),
            _ => None,
        })
        .collect()
}

#[test]
fn pressure_compacts_oldest_with_audit_refs_and_records_context_inject() {
    let _guard = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let dump = dir.path().join("prompts.txt");
    unsafe {
        std::env::set_var("REC_DUMP", &dump);
        std::env::set_var("REC_MODE", "pressure");
    }
    let mut l = rig(dir.path(), log.path());
    let stream = l.stream_id();
    // recmodel writes the secret ONLY after a prompt carries COMPACTED
    let r = l.run_mission("task-0").unwrap();
    assert!(r.passed, "mission passes only if the compacted summary reached the window");

    let prompts = std::fs::read_to_string(&dump).unwrap();
    let summary = prompts
        .lines()
        .find(|l| l.contains("COMPACTED"))
        .expect("a compaction summary line in some prompt");
    assert!(
        summary.contains("seq"),
        "summary links back to the source event range: {summary}"
    );

    let injects = context_injects(log.path(), stream);
    assert!(
        injects.iter().any(|b| b.contains("why=pressure")),
        "context_inject recorded with why=pressure: {injects:?}"
    );
}

#[test]
fn no_pressure_no_compaction() {
    let _guard = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let dump = dir.path().join("prompts.txt");
    unsafe {
        std::env::set_var("REC_DUMP", &dump);
        std::env::set_var("REC_MODE", "small");
    }
    let mut l = rig(dir.path(), log.path());
    let stream = l.stream_id();
    let r = l.run_mission("task-0").unwrap();
    assert!(r.passed, "small mission passes on the marker alone");

    let prompts = std::fs::read_to_string(&dump).unwrap();
    assert!(
        !prompts.contains("COMPACTED"),
        "under the watermark nothing is compacted"
    );
    assert!(
        context_injects(log.path(), stream).is_empty(),
        "no context_inject without pressure"
    );
}
