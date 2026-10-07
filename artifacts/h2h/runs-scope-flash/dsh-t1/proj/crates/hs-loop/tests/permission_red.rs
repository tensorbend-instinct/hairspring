//! Permission ask/auto enforced in the dispatcher: with `ask`, a mutating call
//! publishes a question and runs only after the user answers allow.
use hs_loop::*;
use serde_json::json;

const TERMEXEC: &str = env!("CARGO_BIN_EXE_hs-plugin-termexec");
const PROBE: &str = env!("CARGO_BIN_EXE_hs-plugin-probe");
const SEQMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-seqmodel");
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn rig(dir: &std::path::Path, log: &std::path::Path) -> InnerLoop {
    let config = dir.join("hairspring.toml");
    std::fs::write(&config, format!("[[tools]]\nname = \"term.exec\"\ncommand = [\"{TERMEXEC}\"]\nsubjects = [\"*\"]\n\n[[tools]]\nname = \"probe.read\"\ncommand = [\"{PROBE}\"]\nsubjects = [\"*\"]\n\n[[models]]\nname = \"seqmodel\"\ncommand = [\"{SEQMODEL}\"]\ndefault = true\n")).unwrap();
    InnerLoop::new(hs_kernel::Kernel::load(&config).unwrap(), log, true, 4).unwrap()
}

fn answer_later(ask: std::path::PathBuf, n: u32, ans: &'static str) -> std::thread::JoinHandle<serde_json::Value> {
    std::thread::spawn(move || {
        for _ in 0..200 {
            if ask.join(format!("q-{n}.json")).exists() {
                let q: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(ask.join(format!("q-{n}.json"))).unwrap()).unwrap();
                std::fs::write(ask.join(format!("a-{n}.json")), json!({"answer": ans}).to_string()).unwrap();
                return q;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        serde_json::Value::Null
    })
}

#[test]
fn q1_ask_blocks_until_allowed_then_runs_and_deny_refuses() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (wd, ask, d, l) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let pf = wd.path().join("permission");
    unsafe {
        std::env::set_var("HS_TERM_WORKDIR", wd.path());
        std::env::set_var("HS_ASK_DIR", ask.path());
        std::env::set_var("HS_PERMISSION_FILE", &pf);
        std::env::set_var("HS_ASK_TIMEOUT_SECS", "5");
    }
    let mut lp = rig(d.path(), l.path());
    // auto (no file): runs with no question
    let out = lp.dispatch_tool_for_test("term.exec", &json!({"command":"echo auto"})).unwrap();
    assert!(!out.output.to_string().contains("permission"), "{:?}", out.output);
    assert!(!ask.path().join("q-1.json").exists());
    // ask + allow
    std::fs::write(&pf, "ask").unwrap();
    let h = answer_later(ask.path().to_path_buf(), 1, "allow");
    let out = lp.dispatch_tool_for_test("term.exec", &json!({"command":"echo allowed"})).unwrap();
    let q = h.join().unwrap();
    assert!(q["question"].as_str().unwrap().contains("term.exec"), "{q}");
    assert_eq!(q["options"][0], "allow");
    assert!(out.output.to_string().contains("allowed"), "{:?}", out.output);
    // reads never ask
    let out = lp.dispatch_tool_for_test("probe.read", &json!({"k":"v"})).unwrap();
    assert!(!out.output.to_string().contains("permission"), "{:?}", out.output);
    assert!(!ask.path().join("q-2.json").exists());
    // ask + deny: refused, command never runs
    let h = answer_later(ask.path().to_path_buf(), 2, "deny");
    let marker = wd.path().join("ran.txt");
    let out = lp.dispatch_tool_for_test("term.exec", &json!({"command": format!("touch {}", marker.display())})).unwrap();
    h.join().unwrap();
    assert!(out.output["error"].as_str().unwrap().contains("permission denied"), "{:?}", out.output);
    assert!(!marker.exists());
    unsafe {
        for k in ["HS_TERM_WORKDIR", "HS_ASK_DIR", "HS_PERMISSION_FILE", "HS_ASK_TIMEOUT_SECS"] { std::env::remove_var(k); }
    }
}

#[test]
fn q2_no_answer_times_out_as_denied() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (wd, ask, d, l) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let pf = wd.path().join("permission");
    std::fs::write(&pf, "ask").unwrap();
    unsafe {
        std::env::set_var("HS_TERM_WORKDIR", wd.path());
        std::env::set_var("HS_ASK_DIR", ask.path());
        std::env::set_var("HS_PERMISSION_FILE", &pf);
        std::env::set_var("HS_ASK_TIMEOUT_SECS", "1");
    }
    let mut lp = rig(d.path(), l.path());
    let out = lp.dispatch_tool_for_test("term.exec", &json!({"command":"echo x"})).unwrap();
    assert!(out.output["error"].as_str().unwrap().contains("permission denied"), "{:?}", out.output);
    unsafe {
        for k in ["HS_TERM_WORKDIR", "HS_ASK_DIR", "HS_PERMISSION_FILE", "HS_ASK_TIMEOUT_SECS"] { std::env::remove_var(k); }
    }
}
