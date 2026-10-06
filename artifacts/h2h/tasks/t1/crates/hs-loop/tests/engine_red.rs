//! In-process engine (desktop app backend): events stream as JSON while a
//! mission runs, the result and sessions come back as JSON, no CLI shell-out.
use hs_loop::engine::Engine;
use serde_json::Value;
use std::sync::{Arc, Mutex};

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-liechecker");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn config(dir: &std::path::Path) -> std::path::PathBuf {
    let c = dir.join("hairspring.toml");
    std::fs::write(&c, format!(r#"
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
    c
}

#[test]
fn e1_mission_streams_json_events_and_returns_json_result() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (d, log) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let cfg = config(d.path());
    let ans = d.path().join("a.txt");
    let script = d.path().join("script.jsonl");
    std::fs::write(&script, format!("{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN\"}}}}\n", ans.display())).unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let mut eng = Engine::open(&cfg, log.path(), Some(3)).expect("open");
    let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
    let s2 = Arc::clone(&seen);
    let r = eng.run_goal("write the token", move |ev| s2.lock().unwrap().push(ev)).expect("run");
    assert!(r["steps"].as_u64().unwrap() >= 1, "{r}");
    assert!(r["outcome"].is_string() && r["stream_id"].is_string(), "{r}");
    let evs = seen.lock().unwrap();
    let types: Vec<&str> = evs.iter().filter_map(|e| e["type"].as_str()).collect();
    assert!(types.contains(&"step"), "{types:?}");
    assert!(types.contains(&"tool_start") && types.contains(&"tool_end"), "{types:?}");
    let ts = evs.iter().find(|e| e["type"] == "tool_start").unwrap();
    assert_eq!(ts["plugin"], "answer.write");
}

#[test]
fn e2_sessions_vitals_models_are_json() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (d, log) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let cfg = config(d.path());
    let ans = d.path().join("a.txt");
    let script = d.path().join("script.jsonl");
    std::fs::write(&script, format!("{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"X\"}}}}\n", ans.display())).unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let mut eng = Engine::open(&cfg, log.path(), Some(3)).unwrap();
    eng.run_goal("hello goal", |_| {}).unwrap();
    let s = eng.sessions();
    assert_eq!(s["groups"][0]["sessions"].as_array().unwrap().len(), 1, "{s}");
    let v = eng.vitals();
    assert_eq!(v["missions"], 1, "{v}");
    assert_eq!(eng.models()[0]["name"], "scripted");
    eng.compact();
}

#[test]
fn e3_ui_event_wire_shapes() {
    use hs_loop::uipaint::UiEvent;
    let j = UiEvent::ToolCallEnd { plugin: "term.exec".into(), ok: true, output_summary: "ok".into(), elapsed_ms: 42 }.to_json();
    assert_eq!(j["type"], "tool_end");
    assert_eq!(j["elapsed_ms"], 42);
    let j = UiEvent::Step { step: 2, max_steps: Some(9) }.to_json();
    assert_eq!((j["step"].as_u64(), j["max_steps"].as_u64()), (Some(2), Some(9)));
    let j = UiEvent::ModelCallCache { cached_tokens: 5, input_tokens: 10 }.to_json();
    assert_eq!(j["cached_tokens"], 5);
}

#[test]
fn e4_export_session_zip() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (d, log) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let cfg = config(d.path());
    let ans = d.path().join("a.txt");
    let script = d.path().join("script.jsonl");
    std::fs::write(&script, format!("{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"X\"}}}}\n", ans.display())).unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let mut eng = Engine::open(&cfg, log.path(), Some(3)).unwrap();
    let r = eng.run_goal("export me", |_| {}).unwrap();
    let id = r["stream_id"].as_str().unwrap().to_string();
    let out = d.path().join("s.zip");
    let n = eng.export_session(&id, &out).unwrap();
    assert!(n >= 1);
    let o = std::process::Command::new("python3").args(["-c", "import zipfile,sys;z=zipfile.ZipFile(sys.argv[1]);assert z.testzip() is None;print(len(z.namelist()))", out.to_str().unwrap()]).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(eng.export_session("not-a-uuid", &out).is_err());
}

#[test]
fn e5_schedule_firing_loop_runs_due_prompts_once() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (d, log, wd) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let cfg = config(d.path());
    let ans = d.path().join("a.txt");
    let script = d.path().join("script.jsonl");
    std::fs::write(&script, format!("{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"X\"}}}}\n", ans.display())).unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let mut eng = Engine::open(&cfg, log.path(), Some(3)).unwrap();
    let c = hs_loop::tools2::schedule(wd.path(), &serde_json::json!({"op":"create","prompt":"nightly check","every_secs":60,"now":1000}));
    assert_eq!(c["ok"], true, "{c}");
    // not due yet
    assert!(eng.fire_due_schedules(wd.path(), 1030).is_empty());
    // due: fires exactly once and runs a real mission
    let fired = eng.fire_due_schedules(wd.path(), 1061);
    assert_eq!(fired.len(), 1, "{fired:?}");
    assert_eq!(fired[0]["prompt"], "nightly check");
    assert!(fired[0]["result"]["stream_id"].is_string(), "{:?}", fired[0]);
    // advanced: same instant does not refire; next period does
    assert!(eng.fire_due_schedules(wd.path(), 1062).is_empty());
    assert_eq!(eng.fire_due_schedules(wd.path(), 1125).len(), 1);
    assert_eq!(eng.vitals()["missions"], 2);
}

#[test]
fn e6_run_workflow_runs_steps_in_order() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (d, log, wd) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let cfg = config(d.path());
    let ans = d.path().join("a.txt");
    let script = d.path().join("script.jsonl");
    std::fs::write(&script, format!("{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"X\"}}}}\n", ans.display())).unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let mut eng = Engine::open(&cfg, log.path(), Some(3)).unwrap();
    let none = eng.run_workflow(wd.path(), "ghost");
    assert_eq!(none["ok"], false);
    hs_loop::tools2::workflow(wd.path(), &serde_json::json!({"op":"define","name":"two","steps":["step one","step two"]}));
    let r = eng.run_workflow(wd.path(), "two");
    assert!(r["ran"].as_u64().unwrap() >= 1, "{r}");
    assert_eq!(eng.vitals()["missions"], r["ran"], "{r}");
}

#[test]
fn e7_modes_narrow_and_restore_the_tool_surface() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (d, log) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let cfg = config(d.path());
    let mut eng = Engine::open(&cfg, log.path(), Some(3)).unwrap();
    let std_names = eng.tool_names();
    assert!(std_names.iter().any(|n| n == "agent.spawn")  && std_names.iter().any(|n| n == "web.search" || n == "memory.recall"), "{std_names:?}");
    eng.set_mode("minimal").unwrap();
    let min = eng.tool_names();
    assert!(!min.iter().any(|n| n == "agent.spawn" || n == "web.search"), "{min:?}");
    assert_eq!(hs_loop::modes::allows("minimal", "answer.submit"), Some(true));
    eng.set_mode("ptc").unwrap();
    let ptc = eng.tool_names();
    assert!(ptc.iter().any(|n| n == "agent.spawn") && ptc.len() > min.len(), "{ptc:?}");
    eng.set_mode("creator").unwrap();
    assert!(!eng.tool_names().iter().any(|n| n == "agent.spawn"));
    eng.set_mode("standard").unwrap();
    assert_eq!(eng.tool_names(), std_names);
    assert!(eng.set_mode("turbo").unwrap_err().contains("unknown mode"));
}

#[test]
fn e_resume_reopens_the_same_stream_and_knows_its_workspace() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (d, log, proj) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let cfg = config(d.path());
    let script = d.path().join("script.jsonl");
    let ans = d.path().join("a.txt");
    std::fs::write(&script, format!("{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"X\"}}}}\n", ans.display())).unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    unsafe { std::env::set_var("HS_PROJECT_ROOT", proj.path()) };
    let id = {
        let mut eng = Engine::open(&cfg, log.path(), Some(3)).unwrap();
        eng.run_goal("first goal", |_| {}).unwrap();
        eng.vitals()["stream_id"].as_str().unwrap().to_string()
    };
    unsafe { std::env::remove_var("HS_PROJECT_ROOT") };
    // the session remembers the folder it ran in
    assert_eq!(hs_loop::appback::session_workspace(log.path(), &id).as_deref(), Some(proj.path().canonicalize().unwrap().to_str().unwrap()));
    let eng = Engine::open_resume(&cfg, log.path(), Some(3), &id).expect("resume");
    assert_eq!(eng.vitals()["stream_id"], id.as_str(), "same stream, history kept");
    assert!(Engine::open_resume(&cfg, log.path(), Some(3), "not-a-uuid").is_err());
}
