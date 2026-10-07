//! Background jobs: start returns at once, output is readable while it runs,
//! list shows status/exit code, kill ends a long job.
use hs_loop::jobs::call;
use serde_json::json;
use std::time::{Duration, Instant};

fn wait_exit(wd: &std::path::Path, id: &str) -> serde_json::Value {
    let t = Instant::now();
    loop {
        let v = call(wd, &json!({"op":"output","id":id}));
        if v["status"] == "exited" || t.elapsed() > Duration::from_secs(10) {
            return v;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn j1_start_returns_immediately_and_output_and_exit_code_are_recorded() {
    let d = tempfile::tempdir().unwrap();
    let t = Instant::now();
    let s = call(d.path(), &json!({"op":"start","command":"sleep 1; echo hello-job; exit 3"}));
    assert!(t.elapsed() < Duration::from_millis(800), "start must not block: {:?}", t.elapsed());
    assert_eq!(s["ok"], true, "{s}");
    let id = s["id"].as_str().unwrap();
    assert_eq!(call(d.path(), &json!({"op":"list"}))["jobs"][0]["status"], "running");
    let v = wait_exit(d.path(), id);
    assert_eq!(v["status"], "exited", "{v}");
    assert_eq!(v["exit_code"], 3);
    assert!(v["output"].as_str().unwrap().contains("hello-job"), "{v}");
}

#[test]
fn j2_kill_stops_a_long_job() {
    let d = tempfile::tempdir().unwrap();
    let s = call(d.path(), &json!({"op":"start","command":"sleep 300"}));
    let id = s["id"].as_str().unwrap().to_string();
    let k = call(d.path(), &json!({"op":"kill","id":id}));
    assert_eq!(k["ok"], true, "{k}");
    let l = call(d.path(), &json!({"op":"list"}));
    assert_eq!(l["jobs"][0]["status"], "exited", "{l}");
}

#[test]
fn j3_unknown_job_and_tail_cap() {
    let d = tempfile::tempdir().unwrap();
    assert!(call(d.path(), &json!({"op":"output","id":"j9"}))["$error"].is_string());
    let s = call(d.path(), &json!({"op":"start","command":"printf 'x%.0s' $(seq 1 500)"}));
    let v = wait_exit(d.path(), s["id"].as_str().unwrap());
    let o = call(d.path(), &json!({"op":"output","id":s["id"],"tail_bytes":100}));
    assert_eq!(o["output"].as_str().unwrap().len(), 100, "{v}");
    assert_eq!(o["truncated"], true);
}

#[test]
fn j4_wired_schema_install_config() {
    let t = hs_loop::toolschema::schema_for("jobs", "apply").expect("jobs schema");
    assert_eq!(t["function"]["parameters"]["required"][0], "op", "{t}");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert!(std::fs::read_to_string(root.join("install.sh")).unwrap().contains("hs-plugin-jobs"));
    let toml = std::fs::read_to_string(root.join("hairspring.example.toml")).unwrap();
    assert!(toml.contains("name = \"jobs\"") && toml.contains("hs-plugin-jobs"));
}
