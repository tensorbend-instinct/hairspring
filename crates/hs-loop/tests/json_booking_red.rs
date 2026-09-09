//! RED (deep-pass hostile review, hs-loop): two stream events are booked
//! with `format!`-built JSON instead of serde.
//!
//! j1: `poll_gateway_tasks` embeds the goal with `{goal:?}` (Rust debug
//!     escapes). Debug escaping is NOT JSON escaping: a DEL/control char
//!     comes out as `\u{7f}` - brace syntax no JSON parser accepts - so a
//!     single mid-run goal carrying one control byte poisons the
//!     canonical record with an unparseable Message payload. Every
//!     booked payload must parse as JSON and round-trip the input.
//!
//! j2: `set_model_override` embeds old/new model names raw into a
//!     `format!` JSON template. A configured model name carrying a
//!     backslash produces an unparseable `CapabilityChange` event on the
//!     session stream. Same rule: booked payloads parse and round-trip.

use hs_loop::repl::load_session;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn write_fixture(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).unwrap();
    // model name with a backslash: legal TOML, legal config - but raw
    // insertion into a JSON template emits the invalid escape `\g`.
    let weird = format!("{}/wrap-weird.sh", dir.display());
    std::fs::write(
        &weird,
        concat!(
            "#!/bin/sh\n",
            "export HS_SCRIPTED_NAME='weird\\gen'\n",
            "exec /mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted\n",
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(&weird, std::fs::Permissions::from_mode(0o755)).unwrap();
    let toml = format!(
        concat!(
            "[[models]]\n",
            "name = \"scripted\"\n",
            "command = [\"/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted\"]\n",
            "default = true\n",
            "subjects = [\"*\"]\n",
            "\n",
            "[[models]]\n",
            "name = 'weird\\gen'\n",
            "command = [\"{0}\"]\n",
            "subjects = [\"*\"]\n",
        ),
        weird,
    );
    std::fs::write(dir.join("hairspring.toml"), toml).unwrap();
    // one prose completion: the mission burns its single step and exits
    // steps_exhausted (we only need the per-step gateway drain to run).
    std::fs::write(dir.join("script.jsonl"), "\"no tool call here\"\n").unwrap();
}

fn booked_payloads(run: &std::path::Path, sid: uuid::Uuid, kind: hs_core::EventKind) -> Vec<String> {
    let reader = hs_log::StreamReader::open(run, sid).unwrap();
    reader
        .events()
        .unwrap()
        .iter()
        .filter(|e| e.kind == kind)
        .filter_map(|e| {
            reader
                .resolve_payload(e)
                .ok()
                .map(|b| String::from_utf8_lossy(&b).into_owned())
        })
        .collect()
}

#[test]
fn j1_gateway_goal_with_control_char_books_valid_json() {
    let _g = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join("json-booking-j1");
    let _ = std::fs::remove_dir_all(&dir);
    write_fixture(&dir);
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl")) };
    let run = dir.join("run");
    std::fs::create_dir_all(&run).unwrap();
    let mut s = load_session(&dir.join("hairspring.toml"), &run, false, 1, None, None).unwrap();
    // A goal injected mid-run whose bytes include DEL (0x7F): the buggy
    // booking wrote it as literally-backslash-u-brace garbage.
    std::fs::write(run.join("task-inbox.txt"), "fix-the\u{7f}-thing\n").unwrap();
    s.set_task_inbox(&run.join("task-inbox.txt"));
    let r = s.run_goal("seed mission").unwrap();
    assert!(!r.passed, "1-step prose completion exhausts: {r:?}");
    let msgs = booked_payloads(&run, s.vitals().stream_id, hs_core::EventKind::Message);
    assert_eq!(
        msgs.len(),
        1,
        "the drained goal books exactly one Message: {msgs:?}"
    );
    let parsed: serde_json::Value = serde_json::from_str(&msgs[0]).unwrap_or_else(|e| {
        panic!(
            "the booked gateway Message must be valid JSON: {e}; payload: {}",
            msgs[0]
        )
    });
    assert_eq!(
        parsed["gateway"].as_str(),
        Some("task_queued"),
        "the booking keeps its schema: {parsed}"
    );
    assert_eq!(
        parsed["goal"].as_str(),
        Some("fix-the\u{7f}-thing"),
        "the goal round-trips byte-exactly: {parsed}"
    );
    assert_eq!(
        s.take_queued_goals(),
        vec!["fix-the\u{7f}-thing".to_string()]
    );
}

#[test]
fn j2_capability_change_books_valid_json() {
    let _g = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join("json-booking-j2");
    let _ = std::fs::remove_dir_all(&dir);
    write_fixture(&dir);
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl")) };
    let run = dir.join("run");
    let mut s = load_session(&dir.join("hairspring.toml"), &run, false, 1, None, None).unwrap();
    s.set_model_override(Some("weird\\gen".to_string())).unwrap();
    let mut s2 = s;
    s2.set_model_override(None).unwrap();
    let evs = booked_payloads(
        &run,
        s2.vitals().stream_id,
        hs_core::EventKind::CapabilityChange,
    );
    assert_eq!(
        evs.len(),
        2,
        "swap-and-restore books two CapabilityChange events: {evs:?}"
    );
    let first: serde_json::Value = serde_json::from_str(&evs[0]).unwrap_or_else(|e| {
        panic!(
            "the booked CapabilityChange must be valid JSON: {e}; payload: {}",
            evs[0]
        )
    });
    assert_eq!(first["capability"].as_str(), Some("model"));
    assert_eq!(first["old_binding"].as_str(), Some("scripted"));
    assert_eq!(first["new_binding"].as_str(), Some("weird\\gen"));
    let second: serde_json::Value = serde_json::from_str(&evs[1]).unwrap_or_else(|e| {
        panic!("restore event must be valid JSON: {e}; payload: {}", evs[1])
    });
    assert_eq!(second["old_binding"].as_str(), Some("weird\\gen"));
    assert_eq!(second["new_binding"].as_str(), Some("scripted"));
}
