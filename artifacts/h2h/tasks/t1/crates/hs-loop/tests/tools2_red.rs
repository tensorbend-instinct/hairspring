use hs_loop::tools2::{present, read_image, schedule, todo};
use serde_json::json;

#[test]
fn t1_todo_write_replaces_list_and_reads_back() {
    let d = tempfile::tempdir().unwrap();
    let r = todo(d.path(), &json!({"todos":[{"content":"a","status":"in_progress"},{"content":"b","status":"pending"}]}));
    assert_eq!(r["counts"]["in_progress"], 1, "{r}");
    let g = todo(d.path(), &json!({}));
    assert_eq!(g["todos"].as_array().unwrap().len(), 2);
    assert!(todo(d.path(), &json!({"todos":[{"content":"x","status":"bogus"}]}))["$error"].is_string());
    assert!(todo(d.path(), &json!({"todos":[{"content":"a","status":"in_progress"},{"content":"b","status":"in_progress"}]}))["$error"].is_string(), "only one in_progress");
}

#[test]
fn t2_present_publishes_file_and_refuses_escape() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("r.md"), "# hi").unwrap();
    let r = present(d.path(), &json!({"path":"r.md","title":"Report"}));
    assert_eq!(r["presented"]["mime"], "text/markdown", "{r}");
    assert_eq!(r["presented"]["bytes"], 4);
    assert!(present(d.path(), &json!({"path":"../etc/passwd"}))["$error"].is_string());
    assert!(present(d.path(), &json!({"path":"missing.md"}))["$error"].is_string());
    assert!(d.path().join(".hs/presented.jsonl").exists());
}

#[test]
fn t3_read_image_reports_dims_and_base64() {
    let d = tempfile::tempdir().unwrap();
    // 1x1 PNG
    let png: Vec<u8> = vec![137,80,78,71,13,10,26,10,0,0,0,13,73,72,68,82,0,0,0,1,0,0,0,1,8,6,0,0,0,31,21,196,137,0,0,0,10,73,68,65,84,120,156,99,0,1,0,0,5,0,1,13,10,45,180,0,0,0,0,73,69,78,68,174,66,96,130];
    std::fs::write(d.path().join("p.png"), &png).unwrap();
    let r = read_image(d.path(), &json!({"path":"p.png"}));
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(1), Some(1)), "{r}");
    assert_eq!(r["mime"], "image/png");
    assert!(r["data_base64"].as_str().unwrap().starts_with("iVBOR"));
    std::fs::write(d.path().join("t.txt"), "x").unwrap();
    assert!(read_image(d.path(), &json!({"path":"t.txt"}))["$error"].is_string());
}

#[test]
fn t4_schedule_crud_and_due() {
    let d = tempfile::tempdir().unwrap();
    let c = schedule(d.path(), &json!({"op":"create","prompt":"check build","every_secs":60,"now":1000}));
    let id = c["schedule"]["id"].as_str().unwrap().to_string();
    assert_eq!(c["schedule"]["next_at"], 1060, "{c}");
    assert_eq!(schedule(d.path(), &json!({"op":"list"}))["schedules"].as_array().unwrap().len(), 1);
    assert_eq!(schedule(d.path(), &json!({"op":"due","now":1059}))["due"].as_array().unwrap().len(), 0);
    let due = schedule(d.path(), &json!({"op":"due","now":1060}));
    assert_eq!(due["due"][0]["prompt"], "check build");
    // due advances the next fire
    assert_eq!(schedule(d.path(), &json!({"op":"list"}))["schedules"][0]["next_at"], 1120);
    let u = schedule(d.path(), &json!({"op":"update","id":id,"every_secs":120,"now":1100}));
    assert_eq!(u["schedule"]["next_at"], 1220, "{u}");
    assert_eq!(schedule(d.path(), &json!({"op":"delete","id":id}))["deleted"], true);
    assert_eq!(schedule(d.path(), &json!({"op":"list"}))["schedules"].as_array().unwrap().len(), 0);
    assert!(schedule(d.path(), &json!({"op":"create","prompt":"x","every_secs":1}))["$error"].is_string(), "min interval");
}

#[test]
fn t5_wired() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ins = std::fs::read_to_string(root.join("install.sh")).unwrap();
    let ex = std::fs::read_to_string(root.join("hairspring.example.toml")).unwrap();
    for (t, b) in [("todo", "hs-plugin-todo"), ("present", "hs-plugin-present"), ("read_image", "hs-plugin-readimage"), ("schedule", "hs-plugin-schedule"), ("workflow", "hs-plugin-workflow"), ("speak", "hs-plugin-speak")] {
        assert!(hs_loop::toolschema::schema_for(t, "apply").is_some(), "schema {t}");
        assert!(ins.contains(b), "install {b}");
        assert!(ex.contains(&format!("name = \"{t}\"")), "example {t}");
    }
}

#[test]
fn t6_workflow_store() {
    let d = tempfile::tempdir().unwrap();
    use serde_json::json;
    let w = |a| hs_loop::tools2::workflow(d.path(), &a);
    assert!(w(json!({"op":"define","name":"x","steps":[]}))["$error"].is_string());
    assert_eq!(w(json!({"op":"define","name":"rel","steps":["build","test"," "]}))["steps"].as_array().unwrap().len(), 2);
    assert_eq!(w(json!({"op":"list"}))["workflows"][0], "rel");
    assert_eq!(w(json!({"op":"get","name":"rel"}))["steps"][1], "test");
    assert_eq!(w(json!({"op":"delete","name":"rel"}))["deleted"], true);
    assert!(w(json!({"op":"get","name":"rel"}))["$error"].is_string());
}

#[test]
fn t7_speak_runs_the_tts_command_and_confines_output() {
    use serde_json::json;
    let d = tempfile::tempdir().unwrap();
    let tts = d.path().join("faketts.sh");
    // a stand-in TTS: writes a valid 1s silent WAV to the --output_file path, echoing stdin length
    std::fs::write(&tts, "#!/bin/sh\nout=\"$4\"\ncat >/dev/null\npython3 -c \"import wave,sys;w=wave.open(sys.argv[1],'wb');w.setnchannels(1);w.setsampwidth(2);w.setframerate(16000);w.writeframes(b'\\0\\0'*16000)\" \"$out\"\n").unwrap();
    std::process::Command::new("chmod").args(["755", tts.to_str().unwrap()]).status().unwrap();
    unsafe { std::env::set_var("HS_TTS_CMD", &tts); std::env::set_var("HS_TTS_MODEL", "m.onnx"); }
    let wd = tempfile::tempdir().unwrap();
    let r = hs_loop::tools2::speak(wd.path(), &json!({"text":"hello there","path":"out/a.wav"}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(r["bytes"].as_u64().unwrap() > 32000, "{r}");
    assert!(wd.path().join("out/a.wav").exists());
    assert!(hs_loop::tools2::speak(wd.path(), &json!({"text":"x","path":"../evil.wav"}))["$error"].is_string());
    assert!(hs_loop::tools2::speak(wd.path(), &json!({"text":"x","path":"/tmp/evil.wav"}))["$error"].is_string());
    assert!(hs_loop::tools2::speak(wd.path(), &json!({"text":"  "}))["$error"].is_string());
    unsafe { std::env::remove_var("HS_TTS_MODEL"); }
    assert!(hs_loop::tools2::speak(wd.path(), &json!({"text":"x"}))["$error"].as_str().unwrap().contains("no voice"));
    unsafe { std::env::remove_var("HS_TTS_CMD"); }
}

/// Live: real piper voice. Run with `--ignored`; needs HS_LIVE_PIPER=<piper bin> and HS_LIVE_VOICE=<model.onnx>.
#[test]
#[ignore = "needs a real piper install"]
fn t8_live_piper_speaks() {
    let (Ok(bin), Ok(model)) = (std::env::var("HS_LIVE_PIPER"), std::env::var("HS_LIVE_VOICE")) else { panic!("set HS_LIVE_PIPER and HS_LIVE_VOICE") };
    unsafe { std::env::set_var("HS_TTS_CMD", bin); std::env::set_var("HS_TTS_MODEL", model); }
    let wd = tempfile::tempdir().unwrap();
    let r = hs_loop::tools2::speak(wd.path(), &serde_json::json!({"text":"Hairspring voice plugin check.","path":"v.wav"}));
    assert_eq!(r["ok"], true, "{r}");
    let w = std::process::Command::new("python3").args(["-c", "import wave,sys;w=wave.open(sys.argv[1]);print(round(w.getnframes()/w.getframerate(),2))", wd.path().join("v.wav").to_str().unwrap()]).output().unwrap();
    let secs: f64 = String::from_utf8_lossy(&w.stdout).trim().parse().unwrap();
    assert!(secs > 0.8, "{secs}");
    println!("LIVE_PIPER bytes={} seconds={secs}", r["bytes"]);
}
