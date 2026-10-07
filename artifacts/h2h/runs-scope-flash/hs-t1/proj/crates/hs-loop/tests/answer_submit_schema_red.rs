//! RED contract: the model-facing answer.submit schema must match what the
//! plugin actually requires in the mode the mission runs in. The plugin has
//! three modes: HS_SWE_WORKSPACE (diff computed by the harness, {path} only),
//! HS_ANSWER_RAW (terminal-bench, {path} only), and the plain REPL mode
//! ({path, summary} - the summary IS the answer content). GLM-5.3 exposed the
//! mismatch live: served the SWE {path}-only schema in REPL mode, it burned
//! 50 steps re-sending exactly what the schema declared.

use hs_loop::toolschema::schema_for;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn required(name: &str) -> Vec<String> {
    let t = schema_for(name, "apply").expect("answer.submit schema");
    t["function"]["parameters"]["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn raw_mode_requires_summary() {
    let _g = ENV_LOCK.lock().unwrap();
    let prev_raw = std::env::var("HS_ANSWER_RAW").ok();
    unsafe { std::env::set_var("HS_ANSWER_RAW", "1") };
    let r = required("answer.submit");
    match prev_raw {
        Some(v) => unsafe { std::env::set_var("HS_ANSWER_RAW", v) },
        None => unsafe { std::env::remove_var("HS_ANSWER_RAW") },
    }
    assert_eq!(r, vec!["path".to_string(), "summary".to_string()], "{r:?}");
}

#[test]
fn swe_mode_stays_path_only() {
    let _g = ENV_LOCK.lock().unwrap();
    let prev = std::env::var("HS_ANSWER_RAW").ok();
    unsafe { std::env::remove_var("HS_ANSWER_RAW") };
    let r = required("answer.submit");
    if let Some(v) = prev {
        unsafe { std::env::set_var("HS_ANSWER_RAW", v) };
    }
    assert_eq!(r, vec!["path".to_string()], "{r:?}");
}

#[test]
fn registry_path_serves_summary_in_raw_mode() {
    let _g = ENV_LOCK.lock().unwrap();
    let prev_raw = std::env::var("HS_ANSWER_RAW").ok();
    unsafe { std::env::set_var("HS_ANSWER_RAW", "1") };
    let tools = hs_loop::toolschema::schemas_for_registry(
        &["answer.submit".to_string()],
        &[],
        "applypatch",
    );
    match prev_raw {
        Some(v) => unsafe { std::env::set_var("HS_ANSWER_RAW", v) },
        None => unsafe { std::env::remove_var("HS_ANSWER_RAW") },
    }
    let req: Vec<String> = tools[0]["function"]["parameters"]["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(req, vec!["path".to_string(), "summary".to_string()], "{req:?}");
}
