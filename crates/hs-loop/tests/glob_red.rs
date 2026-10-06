use hs_loop::tools2::glob;
use serde_json::json;

#[test]
fn g1_matches_patterns_sorted_and_respects_gitignore() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    std::fs::create_dir_all(p.join("src/sub")).unwrap();
    std::fs::create_dir_all(p.join("target")).unwrap();
    std::fs::write(p.join("src/a.rs"), "").unwrap();
    std::fs::write(p.join("src/sub/b.rs"), "").unwrap();
    std::fs::write(p.join("src/c.txt"), "").unwrap();
    std::fs::write(p.join("target/z.rs"), "").unwrap();
    std::fs::write(p.join(".gitignore"), "target/\n").unwrap();
    let r = glob(p, &json!({"pattern":"**/*.rs"}));
    let f: Vec<&str> = r["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(f, vec!["src/a.rs", "src/sub/b.rs"], "{r}");
    let r = glob(p, &json!({"pattern":"*.txt","path":"src"}));
    assert_eq!(r["files"][0], "c.txt", "{r}");
}

#[test]
fn g2_guards_and_cap() {
    let d = tempfile::tempdir().unwrap();
    assert!(glob(d.path(), &json!({}))["$error"].is_string());
    assert!(glob(d.path(), &json!({"pattern":"*","path":"../"}))["$error"].is_string());
    for i in 0..30 {
        std::fs::write(d.path().join(format!("f{i:02}.x")), "").unwrap();
    }
    let r = glob(d.path(), &json!({"pattern":"*.x","limit":10}));
    assert_eq!(r["files"].as_array().unwrap().len(), 10);
    assert_eq!(r["truncated"], true);
}

#[test]
fn g3_wired() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert!(hs_loop::toolschema::schema_for("glob", "apply").is_some());
    assert!(std::fs::read_to_string(root.join("install.sh")).unwrap().contains("hs-plugin-glob"));
    assert!(std::fs::read_to_string(root.join("hairspring.example.toml")).unwrap().contains("name = \"glob\""));
}
