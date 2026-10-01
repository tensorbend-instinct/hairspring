//! RED: conservative schema-guided argument coercion (Hermes
//! arg_coercion): models often send "5" for an integer, "true" for a
//! boolean, or a JSON string for an array/object. Coerce ONLY when the
//! schema says so and the conversion is exact and lossless; never guess.
use hs_loop::argcoerce::coerce;
use serde_json::json;

fn schema() -> serde_json::Value {
    json!({"type":"object","properties":{
        "k":{"type":"integer"},"x":{"type":"number"},"flag":{"type":"boolean"},
        "items":{"type":"array","items":{"type":"integer"}},
        "obj":{"type":"object","properties":{"n":{"type":"integer"}}},
        "s":{"type":"string"}}})
}

#[test]
fn coerces_exact_string_scalars() {
    let (v, n) = coerce(&schema(), &json!({"k":"5","x":"1.5","flag":"true","s":"7"}));
    assert_eq!(v, json!({"k":5,"x":1.5,"flag":true,"s":"7"}));
    assert_eq!(n, 3);
}

#[test]
fn coerces_json_string_containers_and_nested_fields() {
    let (v, n) = coerce(&schema(), &json!({"items":"[1,\"2\",3]","obj":"{\"n\":\"4\"}"}));
    assert_eq!(v, json!({"items":[1,2,3],"obj":{"n":4}}));
    assert!(n >= 2);
}

#[test]
fn never_guesses_or_loses_information() {
    let a = json!({"k":"5.5","x":"abc","flag":"yes","items":"not json","obj":"[1]","k2":"9"});
    let (v, n) = coerce(&schema(), &a);
    assert_eq!(v, a, "inexact or unsupported values must pass through untouched");
    assert_eq!(n, 0);
    // large ints beyond i64, leading zeros-ambiguity and whitespace stay as-is
    let b = json!({"k":"99999999999999999999","flag":"True "});
    assert_eq!(coerce(&schema(), &b).0, b);
    // already-correct and non-object args are untouched
    let c = json!({"k":5,"flag":false});
    assert_eq!(coerce(&schema(), &c), (c.clone(), 0));
    assert_eq!(coerce(&schema(), &json!("str")), (json!("str"), 0));
}

#[test]
fn loop_coerces_string_args_before_dispatch() {
    const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
    const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
    const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");
    unsafe { std::env::set_var("HS_SCRIPTED_PROMPT_AWARE", "1") };
    let dir = std::env::temp_dir().join(format!("hscoerce-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let log = dir.join("log");
    std::fs::create_dir_all(&log).unwrap();
    let mission = "task-9";
    let answer = log.join("work").join(mission).join("answer.txt");
    let script = dir.join("script.jsonl");
    std::fs::write(&script, [
        json!({"tool":"memory.recall","args":{"k":"2"}}).to_string(),
        json!({"tool":"answer.write","args":{"path": answer.display().to_string(), "content":"TOKEN-9-SECRET"}}).to_string(),
    ].join("\n")).unwrap();
    let config = dir.join("hairspring.toml");
    std::fs::write(&config, format!(r#"
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
command = ["/bin/sh", "-c", "HS_SCRIPTED_NAME=scripted HS_SEQMODEL_SCRIPT={} exec {SCRIPTED}"]
default = true
subjects = ["*"]
"#, script.display())).unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = hs_loop::InnerLoop::new(kernel, &log, true, 6).unwrap();
    l.set_memory_db(&log.join("memory.db"));
    l.set_tools(json!([hs_loop::toolschema::memory_recall_tool()]));
    let r = l.run_mission(mission).unwrap();
    assert!(r.passed, "{r:?}");
    assert_eq!(l.arg_coercions(), 1, "k:\"2\" must be coerced to 2 before dispatch");
    let _ = std::fs::remove_dir_all(&dir);
}
