use hs_loop::goal_state::call;
use serde_json::json;

#[test]
fn g1_create_get_update_lifecycle() {
    let d = tempfile::tempdir().unwrap();
    assert!(call(d.path(), &json!({"op":"get"}))["goal"].is_null());
    let c = call(d.path(), &json!({"op":"create","objective":"ship feature X","acceptance":"tests green"}));
    assert_eq!(c["goal"]["status"], "active", "{c}");
    let u = call(d.path(), &json!({"op":"update","note":"parser done"}));
    assert_eq!(u["goal"]["notes"][0], "parser done");
    let u = call(d.path(), &json!({"op":"update","status":"done"}));
    assert_eq!(u["goal"]["status"], "done");
    // persists for a fresh reader (new process / restart)
    assert_eq!(call(d.path(), &json!({"op":"get"}))["goal"]["objective"], "ship feature X");
}

#[test]
fn g2_guards() {
    let d = tempfile::tempdir().unwrap();
    assert!(call(d.path(), &json!({"op":"update","status":"done"}))["$error"].is_string(), "update with no goal");
    assert!(call(d.path(), &json!({"op":"create"}))["$error"].is_string());
    call(d.path(), &json!({"op":"create","objective":"a"}));
    assert!(call(d.path(), &json!({"op":"create","objective":"b"}))["$error"].as_str().unwrap().contains("active goal"));
    assert!(call(d.path(), &json!({"op":"update","status":"bogus"}))["$error"].is_string());
}

#[test]
fn g3_wired() {
    let t = hs_loop::toolschema::schema_for("goal", "apply").expect("schema");
    assert_eq!(t["function"]["parameters"]["required"][0], "op");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert!(std::fs::read_to_string(root.join("install.sh")).unwrap().contains("hs-plugin-goaltool"));
    assert!(std::fs::read_to_string(root.join("hairspring.example.toml")).unwrap().contains("name = \"goal\""));
}
