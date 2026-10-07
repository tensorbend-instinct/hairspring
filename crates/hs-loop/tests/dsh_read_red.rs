use serde_json::json;
#[test]
fn exact_read_schema_matches_captured_wire(){let fixture:serde_json::Value=serde_json::from_str(include_str!("../../../artifacts/tool-diff/dsh-wire-tools.json")).unwrap();assert_eq!(hs_loop::dshtools::schema("read").unwrap(),fixture["tools"].as_array().unwrap().iter().find(|t|t["name"]=="read").unwrap().clone());}
#[test]
fn read_has_numbered_window_and_eof(){let d=tempfile::tempdir().unwrap();std::fs::write(d.path().join("a"),"one\ntwo\nthree\n").unwrap();let v=hs_loop::dshtools::read(d.path(),&json!({"file_path":"a","offset":2,"limit":1})).unwrap();assert_eq!(v["lines"],json!([{"number":2,"text":"two"}]));assert!(v["text"].as_str().unwrap().contains("Use offset=3 to continue."));let v=hs_loop::dshtools::read(d.path(),&json!({"file_path":"a","offset":3})).unwrap();assert!(v["text"].as_str().unwrap().contains("End of file - total 3 lines"));}
#[test]
fn read_rejects_invalid_types_unknown_fields_and_escapes(){let d=tempfile::tempdir().unwrap();std::fs::write(d.path().join("a"),"safe").unwrap();for v in [json!({"file_path":"a","offset":0}),json!({"file_path":"a","limit":2001}),json!({"file_path":"a","offset":1.5}),json!({"file_path":"a","extra":true}),json!({"file_path":"../secret"}),json!({"file_path":3})]{assert!(hs_loop::dshtools::read(d.path(),&v).is_err(),"{v}");}}
#[test]
fn read_caps_utf16_line_length_and_bytes(){let d=tempfile::tempdir().unwrap();std::fs::write(d.path().join("a"),"🙂".repeat(1100)).unwrap();let v=hs_loop::dshtools::read(d.path(),&json!({"file_path":"a"})).unwrap();let line=v["lines"][0]["text"].as_str().unwrap();assert!(line.starts_with(&"🙂".repeat(1000)));assert!(line.ends_with("... (line truncated to 2000 chars)"));std::fs::write(d.path().join("a"),("a".repeat(2000)+"\n").repeat(40)).unwrap();let v=hs_loop::dshtools::read(d.path(),&json!({"file_path":"a"})).unwrap();assert_eq!(v["truncated_by_bytes"],true);}
#[cfg(unix)]
#[test]
fn read_cannot_follow_outside_symlink(){let d=tempfile::tempdir().unwrap();let outside=tempfile::tempdir().unwrap();std::fs::write(outside.path().join("s"),"secret").unwrap();std::os::unix::fs::symlink(outside.path().join("s"),d.path().join("a")).unwrap();assert!(hs_loop::dshtools::read(d.path(),&json!({"file_path":"a"})).is_err());}
#[test]
fn past_eof_is_error_but_empty_file_offset_one_is_valid(){let d=tempfile::tempdir().unwrap();std::fs::write(d.path().join("a"),"one\n").unwrap();assert!(hs_loop::dshtools::read(d.path(),&json!({"file_path":"a","offset":2})).is_err());std::fs::write(d.path().join("a"),"").unwrap();assert!(hs_loop::dshtools::read(d.path(),&json!({"file_path":"a"})).is_ok());}
#[test]
fn utf8_errors_outside_selected_window_are_not_hidden(){let d=tempfile::tempdir().unwrap();std::fs::write(d.path().join("a"),b"first\n\xff").unwrap();assert!(hs_loop::dshtools::read(d.path(),&json!({"file_path":"a","limit":1})).is_err());}
