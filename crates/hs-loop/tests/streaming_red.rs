//! REPL PARITY GAP #3: streaming - model output surfaces INCREMENTALLY
//! while a call is in flight, not in one lump at the end.
//!
//! pi/omp-class REPLs print the model's tokens as they arrive. hs-repl
//! today blocks on the full completion: a 60s DeepSeek call shows nothing
//! until it lands. That is the red.
//!
//! Wire shape (negotiated, backward-compatible): when the kernel has a
//! delta sink registered it adds "stream_deltas": true to model.call
//! params; a streaming-capable plugin then emits interstitial
//! {"id":N,"delta":"..."} frames before the final {"id":N,"result":...};
//! the kernel forwards each delta to the sink and keeps reading. A plugin
//! that ignores the flag (every pre-streaming plugin) is unaffected, and
//! a kernel with no sink never sets the flag - old pairs interoperate.

use hs_core::EventKind;
use hs_loop::*;

// HS_SEQMODEL_SCRIPT / HS_SEQMODEL_DELTAS are process-global: serialize.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");

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
name = "scripted"
command = ["{SCRIPTED}"]
default = true
"#
        ),
    )
    .unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    InnerLoop::new(kernel, log, true, max_steps).unwrap()
}

fn write_script(dir: &std::path::Path, lines: &[serde_json::Value]) -> std::path::PathBuf {
    let p = dir.join("script.jsonl");
    std::fs::write(
        &p,
        lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    p
}

fn completions(log: &std::path::Path, stream: uuid::Uuid) -> Vec<String> {
    let reader = hs_log::StreamReader::open(log, stream).unwrap();
    reader
        .events()
        .unwrap()
        .iter()
        .filter(|e| e.kind == EventKind::ModelCall)
        .map(|e| {
            let body = reader
                .resolve_payload(e)
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();
            let v: serde_json::Value = serde_json::from_str(&body).unwrap();
            v["completion"].as_str().unwrap_or("").to_string()
        })
        .collect()
}

#[test]
fn deltas_stream_to_sink_in_order_and_assemble_to_the_completion() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let answer_path = log.path().join("work").join("task-0").join("answer.txt");
    let lines = vec![
        serde_json::json!({"tool":"answer.write","args":{"path":answer_path.display().to_string(),"content":"one"}}),
        serde_json::json!({"tool":"answer.write","args":{"path":answer_path.display().to_string(),"content":"two"}}),
    ];
    let script = write_script(dir.path(), &lines);
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    unsafe { std::env::set_var("HS_SEQMODEL_DELTAS", "1") };

    let seen: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let seen2 = seen.clone();
    let mut l = rig(dir.path(), log.path(), 2);
    l.set_delta_sink(Box::new(move |d: &str| {
        seen2.lock().unwrap().push(d.to_string());
    }));
    let stream = l.stream_id();
    let r = l.run_mission("task-0").unwrap();
    assert_eq!(r.steps, 2, "mission unaffected by streaming: {r:?}");

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 6, "3 deltas per model call x 2 calls: {seen:?}");
    // deltas arrive in emission order and assemble to each completion
    let comps = completions(log.path(), stream);
    assert_eq!(comps.len(), 2);
    let first: String = seen[0..3].concat();
    let second: String = seen[3..6].concat();
    assert_eq!(first, comps[0], "call-1 deltas assemble to its completion");
    assert_eq!(second, comps[1], "call-2 deltas assemble to its completion");
    // the fixture splits in thirds: parts are non-empty and ordered
    assert!(seen.iter().all(|d| !d.is_empty()), "no empty deltas: {seen:?}");
}

#[test]
fn no_sink_means_no_streaming_negotiation_and_a_clean_mission() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let answer_path = log.path().join("work").join("task-0").join("answer.txt");
    let lines = vec![
        serde_json::json!({"tool":"answer.write","args":{"path":answer_path.display().to_string(),"content":"one"}}),
    ];
    let script = write_script(dir.path(), &lines);
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    unsafe { std::env::set_var("HS_SEQMODEL_DELTAS", "1") };

    let mut l = rig(dir.path(), log.path(), 1);
    // no sink registered: the kernel must NOT set stream_deltas, so the
    // fixture emits no delta frames; the round trip is the classic one.
    let r = l.run_mission("task-0").unwrap();
    assert_eq!(r.steps, 1, "{r:?}");
    let comps = completions(log.path(), l.stream_id());
    assert_eq!(comps.len(), 1);
    assert!(comps[0].contains("answer.write"), "completion intact: {comps:?}");
}
