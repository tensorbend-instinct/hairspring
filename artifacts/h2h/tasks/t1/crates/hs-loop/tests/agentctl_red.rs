use hs_loop::agentctl::{interrupt, list, send};
use serde_json::json;

fn marker(root: &std::path::Path, id: &str, done: bool) {
    let d = root.join("swarm");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join(format!("{id}.spawn.json")), json!({"child_stream_id": id, "mission": format!("m-{id}"), "owner_pid": std::process::id(), "owner_start_ticks": 0}).to_string()).unwrap();
    if done {
        std::fs::write(d.join(format!("{id}.report.json")), json!({"passed": true, "steps": 3}).to_string()).unwrap();
    }
}

#[test]
fn a1_list_reports_running_and_done_children() {
    let d = tempfile::tempdir().unwrap();
    marker(d.path(), "c1", false);
    marker(d.path(), "c2", true);
    let r = list(d.path());
    let a = r["agents"].as_array().unwrap();
    assert_eq!(a.len(), 2, "{r}");
    let st = |id: &str| a.iter().find(|x| x["child_stream_id"] == id).unwrap()["state"].clone();
    assert_eq!(st("c2"), "done");
    assert_ne!(st("c1"), "done");
    assert_eq!(a.iter().find(|x| x["child_stream_id"] == "c2").unwrap()["mission"], "m-c2");
    assert!(list(&d.path().join("nope"))["agents"].as_array().unwrap().is_empty());
}

#[test]
fn a2_send_appends_to_the_child_inbox_and_interrupt_sets_the_flag() {
    let d = tempfile::tempdir().unwrap();
    marker(d.path(), "c1", false);
    assert_eq!(send(d.path(), "c1", "use the cache")["ok"], true);
    assert_eq!(send(d.path(), "c1", "and add tests")["ok"], true);
    let inbox = std::fs::read_to_string(d.path().join("swarm/c1.inbox")).unwrap();
    assert_eq!(inbox.lines().collect::<Vec<_>>(), vec!["use the cache", "and add tests"]);
    assert_eq!(interrupt(d.path(), "c1")["ok"], true);
    assert!(d.path().join("swarm/c1.interrupt").exists());
}

#[test]
fn a3_guards() {
    let d = tempfile::tempdir().unwrap();
    marker(d.path(), "c1", true);
    assert!(send(d.path(), "ghost", "x")["$error"].is_string(), "unknown child");
    assert!(send(d.path(), "c1", "x")["$error"].as_str().unwrap().contains("finished"), "done child");
    assert!(send(d.path(), "../c1", "x")["$error"].is_string(), "path traversal");
    assert!(interrupt(d.path(), "ghost")["$error"].is_string());
    assert!(send(d.path(), "c1", "  ")["$error"].is_string());
}

#[test]
fn a4_wired() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ins = std::fs::read_to_string(root.join("install.sh")).unwrap();
    let ex = std::fs::read_to_string(root.join("hairspring.example.toml")).unwrap();
    let repl = std::fs::read_to_string(root.join("crates/hs-loop/src/repl.rs")).unwrap();
    for f in ["agent_list_tool", "agent_send_tool", "agent_interrupt_tool"] {
        assert_eq!(repl.matches(&format!("toolschema::{f}()")).count(), 2, "repl offers {f} on both surfaces");
    }
    assert_eq!(hs_loop::toolschema::agent_send_tool()["function"]["name"], "agent.send");
    for (t, b) in [("agent.list", "hs-plugin-agentlist"), ("agent.send", "hs-plugin-agentsend"), ("agent.interrupt", "hs-plugin-agentstop")] {
        let _ = t;
        assert!(ins.contains(b), "install {b}");
        assert!(ex.contains(&format!("name = \"{t}\"")), "example {t}");
    }
}
