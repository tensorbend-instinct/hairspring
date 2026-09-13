//! B3 RED (v5 spec 2.5 + section-6 outer loop, checklist item 2.5):
//! the gateway adds tasks MID-RUN - redirect (steering) and cancel
//! (interrupt) already exist; task injection did not. The spec outer
//! loop: "event = wait on gateway traffic | ...); handle(event)" - redirect,
//! cancel / add / rebudget, mid-run".
//!
//! g1: while mission A runs, a goal dropped into the session's task
//!     inbox (the file-ingress half of the gateway, sibling to
//!     steering.txt) is drained at a step boundary, BOOKED on the
//!     stream as gateway traffic (`EventKind::Message`, kind 12 - the
//!     spec's "message # gateway traffic"), queued, and runs after
//!     mission A closes - zero operator interleaving, both missions
//!     on the canonical record.

use hs_loop::repl::load_session;

fn write_fixture(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).unwrap();
    let toml = r#"
[[tools]]
name = "answer.submit"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-answersubmit"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-liechecker"]
subjects = ["*"]

[[models]]
name = "scripted"
command = ["/bin/sh", "-c", "HS_SEQMODEL_DELAY_MS=300 exec /mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted"]
default = true
subjects = ["*"]
"#;
    std::fs::write(dir.join("hairspring.toml"), toml).unwrap();
    // task-1: wrong answer first (mission A keeps running, inbox drains
    // mid-mission), then the secret; verifier sacrificials after each
    // green checker (the adversarial audit consumes one call).
    std::fs::write(
        dir.join("script.jsonl"),
        concat!(
            "{\"tool\":\"answer.submit\",\"args\":{\"path\":\"{RUN}/work/task-1/answer.txt\",\"content\":\"WRONG-1\"}}\n",
            "{\"tool\":\"answer.submit\",\"args\":{\"path\":\"{RUN}/work/task-1/answer.txt\",\"content\":\"TOKEN-1-SECRET\"}}\n",
            "\"nothing further to audit\"\n",
            "{\"tool\":\"answer.submit\",\"args\":{\"path\":\"{RUN}/work/task-2/answer.txt\",\"content\":\"TOKEN-2-SECRET\"}}\n",
            "\"nothing further to audit\"\n",
        ).replace("{RUN}", &dir.join("run").display().to_string()),
    )
    .unwrap();
}

#[test]
fn g1_task_added_mid_run_is_booked_queued_and_run() {
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_ANSWER_RAW", "1") };
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SCRIPTED_PROMPT_AWARE", "1") };
    let dir = std::env::temp_dir().join("gateway-add-g1");
    let _ = std::fs::remove_dir_all(&dir);
    write_fixture(&dir);
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl")) };
    let run = dir.join("run");
    let mut s = load_session(&dir.join("hairspring.toml"), &run, false, Some(8), None, None).unwrap();
    let task_inbox = run.join("task-inbox.txt");
    s.set_task_inbox(&task_inbox);
    // Gateway add DURING mission A: the model sleeps 300ms per call; the
    // file lands ~50ms in, mid-mission, and drains at a step boundary.
    let writer = {
        let p = task_inbox.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            std::fs::write(p, "task-2\n").unwrap();
        })
    };
    let a = s.run_goal("task-1").unwrap();
    writer.join().unwrap();
    assert!(a.passed, "mission A passes: {a:?}");
    let queued = s.take_queued_goals();
    assert_eq!(
        queued,
        vec!["task-2".to_string()],
        "the mid-run injected task is queued for after close"
    );
    // booked as gateway traffic on the canonical record
    let sid = s.vitals().stream_id;
    let reader = hs_log::StreamReader::open(&run, sid).unwrap();
    let mut all = String::new();
    for ev in reader.events().unwrap() {
        all.push_str(&format!("{:?} ", ev.kind));
        if let Ok(b) = reader.resolve_payload(&ev) {
            all.push_str(&String::from_utf8_lossy(&b));
        }
        all.push('\n');
    }
    assert!(
        all.contains("Message") && all.contains("task_queued") && all.contains("task-2"),
        "the add is booked as gateway traffic (Message kind); ledger:\n{all}"
    );
    // after close, the queued task runs: same canonical substrate
    let b = s.run_goal(&queued[0].clone()).unwrap();
    assert!(b.passed, "the added task runs and passes: {b:?}");
}
