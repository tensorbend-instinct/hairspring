//! Time-audit probe (Eric 2026-09-05): measure per-step prompt-assembly cost
//! as the transcript grows. Scripted model fattens the stream with bigread
//! every step; the probe prints assembly_ms per ModelCall from the stream.
use hs_core::EventKind;
use hs_loop::*;

const BIGREAD: &str = env!("CARGO_BIN_EXE_hs-plugin-bigread");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");
const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");

#[test]
fn assembly_cost_vs_transcript_growth() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let script = dir.path().join("script.jsonl");
    std::fs::write(&script, "{\"tool\":\"bigread.read\",\"args\":{\"path\":\"x\"}}\n".repeat(30)).unwrap();
    std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
    let config = dir.path().join("hairspring.toml");
    std::fs::write(&config, format!(r#"
[[tools]]
name = "bigread.read"
command = ["{BIGREAD}"]
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
"#)).unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 30).unwrap();
    let r = l.run_mission("task-0").unwrap();
    let reader = hs_log::StreamReader::open(log.path(), r.stream_id).unwrap();
    let mut rows: Vec<(u64, u64, u64)> = vec![]; // (seq, prompt bytes, assembly_ms)
    for e in reader.events().unwrap() {
        if e.kind != EventKind::ModelCall { continue; }
        let b = reader.resolve_payload(&e).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
        if v["role"].as_str() == Some("verifier") { continue; }
        rows.push((e.seq, v["prompt"].as_str().unwrap_or("").len() as u64, v["assembly_ms"].as_u64().unwrap_or(0)));
    }
    eprintln!("seq,prompt_bytes,assembly_ms");
    for (s, p, a) in &rows {
        eprintln!("{s},{p},{a}");
    }
    assert!(rows.len() >= 25, "enough steps measured: {}", rows.len());
}
