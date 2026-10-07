//! Plan mode: while on, the dispatcher refuses mutating tools before the
//! kernel sees them; reads pass; exit lifts the gate and saves the plan.
use hs_loop::*;
use serde_json::json;

const TERMEXEC: &str = env!("CARGO_BIN_EXE_hs-plugin-termexec");
const PROBE: &str = env!("CARGO_BIN_EXE_hs-plugin-probe");
const SEQMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-seqmodel");
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn rig(dir: &std::path::Path, log: &std::path::Path) -> InnerLoop {
    let config = dir.join("hairspring.toml");
    std::fs::write(&config, format!(r#"
[[tools]]
name = "term.exec"
command = ["{TERMEXEC}"]
subjects = ["*"]

[[tools]]
name = "probe.read"
command = ["{PROBE}"]
subjects = ["*"]

[[models]]
name = "seqmodel"
command = ["{SEQMODEL}"]
default = true
"#)).unwrap();
    InnerLoop::new(hs_kernel::Kernel::load(&config).unwrap(), log, true, 4).unwrap()
}

#[test]
fn p1_gate_blocks_mutation_only_while_on() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let wd = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("HS_TERM_WORKDIR", wd.path()); }
    let (d, l) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut lp = rig(d.path(), l.path());
    // off: the mutating tool reaches the kernel
    let out = lp.dispatch_tool_for_test("term.exec", &json!({"command":"echo hi"})).unwrap();
    assert!(out.output.get("error").is_none_or(|e| !e.as_str().unwrap_or("").contains("plan mode")), "{:?}", out.output);
    // on: blocked, read still passes
    assert_eq!(plan_mode::call(wd.path(), &json!({"op":"enter"}))["plan_mode"], true);
    let out = lp.dispatch_tool_for_test("term.exec", &json!({"command":"rm -rf x"})).unwrap();
    assert!(out.output["error"].as_str().unwrap().contains("plan mode is on"), "{:?}", out.output);
    let out = lp.dispatch_tool_for_test("probe.read", &json!({"k":"v"})).unwrap();
    assert!(out.output.get("error").is_none_or(|e| !e.as_str().unwrap_or("").contains("plan mode")), "{:?}", out.output);
    // exit: saves the plan, lifts the gate
    let r = plan_mode::call(wd.path(), &json!({"op":"exit","plan":"1. fix bug\n2. test"}));
    assert_eq!(r["plan_saved"], true);
    assert!(plan_mode::call(wd.path(), &json!({"op":"status"}))["plan"].as_str().unwrap().contains("fix bug"));
    let out = lp.dispatch_tool_for_test("term.exec", &json!({"command":"echo hi"})).unwrap();
    assert!(out.output.get("error").is_none_or(|e| !e.as_str().unwrap_or("").contains("plan mode")));
    unsafe { std::env::remove_var("HS_TERM_WORKDIR"); }
}

#[test]
fn p2_jobs_start_blocked_but_list_allowed() {
    assert!(plan_mode::mutates("jobs", &json!({"op":"start"})));
    assert!(!plan_mode::mutates("jobs", &json!({"op":"list"})));
    assert!(!plan_mode::mutates("repo.read", &json!({})));
}

#[test]
fn p3_wired_schema_install_config() {
    let t = hs_loop::toolschema::schema_for("plan", "apply").expect("schema");
    assert_eq!(t["function"]["parameters"]["required"][0], "op");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert!(std::fs::read_to_string(root.join("install.sh")).unwrap().contains("hs-plugin-plan"));
    assert!(std::fs::read_to_string(root.join("hairspring.example.toml")).unwrap().contains("name = \"plan\""));
}
