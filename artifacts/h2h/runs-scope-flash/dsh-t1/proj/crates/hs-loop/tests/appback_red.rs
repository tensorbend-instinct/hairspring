//! Backend for the app panels: trajectory, question UI protocol, slash menu,
//! queue-while-busy, plugins list, settings, session age.
use hs_loop::appback;
use hs_loop::engine::Engine;
use serde_json::{json, Value};

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-liechecker");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn config(dir: &std::path::Path) -> std::path::PathBuf {
    let c = dir.join("hairspring.toml");
    std::fs::write(&c, format!("[[tools]]\nname = \"answer.write\"\ncommand = [\"{ANSWER}\"]\nsubjects = [\"*\"]\n\n[[tools]]\nname = \"checker.run\"\ncommand = [\"{CHECKER}\"]\nsubjects = [\"*\"]\n\n[[models]]\nname = \"scripted\"\ncommand = [\"{SCRIPTED}\"]\ndefault = true\n")).unwrap();
    c
}

fn engine(d: &std::path::Path, log: &std::path::Path) -> Engine {
    let script = d.join("script.jsonl");
    std::fs::write(&script, format!("{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"X\"}}}}\n", d.join("a.txt").display())).unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    Engine::open(&config(d), log, Some(3)).unwrap()
}

#[test]
fn a1_question_protocol_pending_then_answered() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("q-1.json"), r#"{"question":"Which db?","options":["pg","sqlite"]}"#).unwrap();
    std::fs::write(d.path().join("q-2.json"), r#"{"question":"Name?","options":null}"#).unwrap();
    let p = appback::pending_questions(d.path());
    assert_eq!(p.len(), 2);
    assert_eq!(p[0]["n"], 1);
    assert_eq!(p[0]["options"][1], "sqlite");
    appback::answer_question(d.path(), 1, "pg").unwrap();
    let a: Value = serde_json::from_str(&std::fs::read_to_string(d.path().join("a-1.json")).unwrap()).unwrap();
    assert_eq!(a["answer"], "pg");
    let p = appback::pending_questions(d.path());
    assert_eq!(p.len(), 1);
    assert_eq!(p[0]["n"], 2);
    assert!(appback::answer_question(d.path(), 99, "x").is_err());
}

#[test]
fn a2_trajectory_counts_a_real_session() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (d, log) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut eng = engine(d.path(), log.path());
    let r = eng.run_goal("trace me", |_| {}).unwrap();
    let id = r["stream_id"].as_str().unwrap();
    let t = eng.trajectory(id).unwrap();
    assert!(t["turns"].as_u64().unwrap() >= 1, "{t}");
    assert!(t["calls"].as_u64().unwrap() >= 1, "{t}");
    assert!(t["events"].as_u64().unwrap() >= t["turns"].as_u64().unwrap() + t["calls"].as_u64().unwrap(), "{t}");
    assert!(t["duration_ms"].is_u64(), "{t}");
    assert!(eng.trajectory("nope").is_err());
}

#[test]
fn a3_sessions_carry_title_and_age() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (d, log) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut eng = engine(d.path(), log.path());
    eng.run_goal("a goal that becomes the title", |_| {}).unwrap();
    let s = eng.sessions();
    let one = &s["groups"][0]["sessions"][0];
    assert!(one["title"].as_str().unwrap().contains("a goal that becomes"), "{s}");
    assert!(!one["title"].as_str().unwrap().contains("ENVIRONMENT"), "{s}");
    assert!(one["age_secs"].as_u64().unwrap() < 120, "{s}");
}

#[test]
fn a4_slash_menu_lists_and_runs() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (d, log) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut eng = engine(d.path(), log.path());
    let names: Vec<String> = appback::slash_commands().iter().map(|c| c["name"].as_str().unwrap().to_string()).collect();
    for want in ["/goal", "/plan", "/feedback", "/compact", "/permission", "/model", "/export", "/add-file", "/mode"] {
        assert!(names.iter().any(|n| n == want), "{want} in {names:?}");
    }
    assert_eq!(appback::slash_filter("/co")[0]["name"], "/compact");
    let r = appback::run_slash(&mut eng, d.path(), "/mode minimal");
    assert_eq!(r["ok"], true, "{r}");
    assert!(!eng.tool_names().iter().any(|n| n == "agent.spawn"));
    let r = appback::run_slash(&mut eng, d.path(), "/mode bogus");
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(appback::run_slash(&mut eng, d.path(), "/compact")["ok"], true);
    // /add-file records the path for the next goal
    let f = d.path().join("notes.txt");
    std::fs::write(&f, "hello file").unwrap();
    let r = appback::run_slash(&mut eng, d.path(), &format!("/add-file {}", f.display()));
    assert_eq!(r["ok"], true, "{r}");
    assert!(r["attached"].as_str().unwrap().contains("hello file"), "{r}");
    assert_eq!(appback::run_slash(&mut eng, d.path(), "/nope")["ok"], false);
}

#[test]
fn a5_queue_while_busy_runs_in_order() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (d, log) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut eng = engine(d.path(), log.path());
    eng.queue_goal("first");
    eng.queue_goal("second");
    assert_eq!(eng.queued(), vec!["first".to_string(), "second".to_string()]);
    let r1 = eng.run_next(|_| {}).unwrap().unwrap();
    assert!(r1["stream_id"].is_string());
    assert_eq!(eng.queued(), vec!["second".to_string()]);
    eng.run_next(|_| {}).unwrap().unwrap();
    assert!(eng.run_next(|_| {}).unwrap().is_none());
    assert_eq!(eng.vitals()["missions"], 2);
}

#[test]
fn a6_plugins_list_from_config_and_settings_roundtrip() {
    let d = tempfile::tempdir().unwrap();
    let cfg = config(d.path());
    let p = appback::plugins(&cfg).unwrap();
    assert_eq!(p["tools"][0]["name"], "answer.write");
    assert_eq!(p["models"][0]["name"], "scripted");
    assert_eq!(p["models"][0]["default"], true);
    let s = appback::settings_get(d.path());
    assert_eq!(s["mode"], "standard");
    appback::settings_set(d.path(), &json!({"mode":"ptc","max_steps":40})).unwrap();
    let s = appback::settings_get(d.path());
    assert_eq!(s["mode"], "ptc");
    assert_eq!(s["max_steps"], 40);
    assert!(appback::settings_set(d.path(), &json!({"mode":"turbo"})).is_err());
    assert!(appback::settings_set(d.path(), &json!({"evil_key":1})).is_err());
}

#[test]
fn project_status_flags_an_empty_folder_and_lists_a_populated_one() {
    let d = tempfile::tempdir().unwrap();
    let s = appback::project_status(d.path());
    assert_eq!(s["empty"], true, "{s}");
    assert!(s["root"].as_str().unwrap().contains(d.path().file_name().unwrap().to_str().unwrap()));
    std::fs::create_dir_all(d.path().join(".hs")).unwrap(); // harness scaffolding does not count
    assert_eq!(appback::project_status(d.path())["empty"], true);
    std::fs::write(d.path().join("a.txt"), "x").unwrap();
    let s = appback::project_status(d.path());
    assert_eq!(s["empty"], false);
    assert_eq!(s["entries"], json!(["a.txt"]));
}

#[test]
fn project_dir_setting_validates_and_persists() {
    let home = tempfile::tempdir().unwrap();
    let proj = tempfile::tempdir().unwrap();
    assert!(appback::settings_set(home.path(), &json!({"project_dir": "/no/such/dir/xyz"})).is_err());
    assert!(appback::settings_set(home.path(), &json!({"project_dir": "relative/path"})).is_err());
    appback::settings_set(home.path(), &json!({"project_dir": proj.path().to_str().unwrap()})).unwrap();
    assert_eq!(appback::settings_get(home.path())["project_dir"], proj.path().to_str().unwrap());
}

// ---- dsh-style workspaces: registered real folders, a browse dialog, switching ----

#[test]
fn workspaces_register_real_folders_dedupe_and_remove() {
    let home = tempfile::tempdir().unwrap();
    let a = tempfile::tempdir().unwrap();
    assert!(appback::workspace_add(home.path(), "/no/such/folder").is_err());
    assert!(appback::workspace_add(home.path(), "rel/path").is_err());
    let w = appback::workspace_add(home.path(), a.path().to_str().unwrap()).unwrap();
    assert_eq!(w["path"], a.path().canonicalize().unwrap().to_str().unwrap());
    assert_eq!(w["name"], a.path().file_name().unwrap().to_str().unwrap());
    // same folder via a trailing-slash alias is the same workspace
    appback::workspace_add(home.path(), &format!("{}/", a.path().display())).unwrap();
    assert_eq!(appback::workspaces_list(home.path()).as_array().unwrap().len(), 1);
    appback::workspace_remove(home.path(), w["path"].as_str().unwrap()).unwrap();
    assert!(appback::workspaces_list(home.path()).as_array().unwrap().is_empty());
}

#[test]
fn browse_lists_only_visible_subfolders_sorted() {
    let d = tempfile::tempdir().unwrap();
    for n in ["zeta", "alpha", ".hidden"] { std::fs::create_dir(d.path().join(n)).unwrap(); }
    std::fs::write(d.path().join("file.txt"), "x").unwrap();
    let b = appback::browse_dir(d.path().to_str().unwrap()).unwrap();
    assert_eq!(b["dirs"], json!(["alpha", "zeta"]));
    assert_eq!(b["path"], d.path().canonicalize().unwrap().to_str().unwrap());
    assert!(b["parent"].is_string());
    assert!(appback::browse_dir("/no/such/folder").is_err());
}

#[test]
fn switching_workspace_sets_the_project_folder_only_for_registered_ones() {
    let home = tempfile::tempdir().unwrap();
    let a = tempfile::tempdir().unwrap();
    let ap = a.path().canonicalize().unwrap();
    assert!(appback::workspace_switch(home.path(), ap.to_str().unwrap()).is_err(), "unregistered");
    appback::workspace_add(home.path(), ap.to_str().unwrap()).unwrap();
    appback::workspace_switch(home.path(), ap.to_str().unwrap()).unwrap();
    assert_eq!(appback::settings_get(home.path())["project_dir"], ap.to_str().unwrap());
}
