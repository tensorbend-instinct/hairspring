//! Structured-messages migration RED (user directive 2026-09-05: "EVERYTHING
//! native, transcript included - efficiency first"). The operator model call
//! is a native chat messages array: a stable mission message, one
//! assistant(tool_calls) + tool pair per history exchange (append-only, the
//! KV-cacheable prefix), and a per-step mutable state tail (ATTEMPT budget,
//! ANSWER_PATH, ARTIFACT, FEEDBACK, LEDGER, MEMORY) as the final user
//! message. No hand-rendered transcript text anywhere. The verifier verdict
//! is a native tool call (verdict.submit), not text-JSON in prose.

use hs_core::{EventKind, Payload};
use hs_loop::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const REPOEXEC: &str = env!("CARGO_BIN_EXE_hs-plugin-repoexec");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");
const VFMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-vfmodel");
const PROBE_BIN: &str = env!("CARGO_BIN_EXE_hs-plugin-probe");

static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn events_of(log: &std::path::Path, stream: uuid::Uuid) -> Vec<(EventKind, String)> {
    let reader = hs_log::StreamReader::open(log, stream).unwrap();
    reader
        .events()
        .unwrap()
        .iter()
        .map(|e| {
            let p = match &e.payload {
                Payload::Inline(b) => String::from_utf8_lossy(b).into_owned(),
                _ => reader
                    .resolve_payload(e)
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_default(),
            };
            (e.kind, p)
        })
        .collect()
}

fn operator_message_payloads(ev: &[(EventKind, String)]) -> Vec<Vec<serde_json::Value>> {
    ev.iter()
        .filter(|(k, p)| *k == EventKind::ModelCall && p.contains("\"messages\""))
        .filter_map(|(_, p)| serde_json::from_str::<serde_json::Value>(p).ok())
        .filter(|v| v["messages"].is_array())
        .map(|v| v["messages"].as_array().unwrap().to_vec())
        .collect()
}

/// The operator call carries a structured messages array from the FIRST
/// step - never a hand-rendered text blob.
#[test]
fn operator_call_is_a_native_messages_array() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let answer = log.path().join("work").join("task-0").join("answer.txt");
    let script = dir.path().join("script.jsonl");
    std::fs::write(
        &script,
        format!(
            "{{\"tool\":\"probe.read\",\"args\":{{\"path\":\"x\"}}}}\n{{\"tool\":\"probe.read\",\"args\":{{\"path\":\"y\"}}}}\n{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-0-SECRET\"}}}}",
            answer.display()
        ),
    )
    .unwrap();
    std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
    let config = dir.path().join("hairspring.toml");
    std::fs::write(
        &config,
        format!(
            r#"
[[tools]]
name = "answer.write"
command = ["{ANSWER}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{CHECKER}"]
subjects = ["*"]

[[tools]]
name = "probe.read"
command = ["{PROBE_BIN}"]
subjects = ["*"]

[[models]]
name = "scripted"
command = ["{SCRIPTED}"]
default = true
"#
        ),
    )
    .unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 10).unwrap();
    let r = l.run_mission("task-0").unwrap();
    assert!(r.passed, "scripted mission passes: {r:?}");
    assert_eq!(r.steps, 3, "two probes then the answer: {r:?}");

    let ev = events_of(log.path(), r.stream_id);
    // no operator ModelCall payload may carry the old text blob field
    for (k, p) in &ev {
        if *k == EventKind::ModelCall
            && !p.contains("ADVERSARIAL VERIFIER")
            && !p.contains("DISTILL")
        {
            let v: serde_json::Value = serde_json::from_str(p).unwrap();
            assert!(
                v.get("prompt").is_none() || v["prompt"].is_null(),
                "operator ModelCall still records a text prompt blob: {p}"
            );
        }
    }
    let steps = operator_message_payloads(&ev);
    assert_eq!(
        steps.len(),
        3,
        "three operator calls carry messages: {}",
        ev.len()
    );

    // step 1: mission message + state tail only
    let s1 = &steps[0];
    assert_eq!(s1.len(), 2, "first step: mission + state tail: {s1:?}");
    assert_eq!(s1[0]["role"], "user");
    assert!(
        s1[0]["content"]
            .as_str()
            .unwrap_or("")
            .starts_with("MISSION: task-0"),
        "mission message first: {}",
        s1[0]
    );
    assert_eq!(
        s1[1]["role"], "user",
        "state tail is the final user message"
    );
    let tail1 = s1[1]["content"].as_str().unwrap_or("");
    assert!(
        tail1.contains("ATTEMPT: step 1 of 10"),
        "budget line in tail: {tail1}"
    );
    assert!(
        tail1.contains("ANSWER_PATH: "),
        "answer path in tail: {tail1}"
    );
    assert!(tail1.contains("LEDGER ("), "ledger in tail: {tail1}");

    // step 2: one history pair appeared between mission and tail
    let s2 = &steps[1];
    assert_eq!(s2.len(), 4, "mission + pair + tail: {s2:?}");
    let asst = &s2[1];
    assert_eq!(asst["role"], "assistant");
    let tcs = asst["tool_calls"].as_array().expect("assistant tool_calls");
    assert_eq!(tcs.len(), 1, "one tool call per reply");
    let name = tcs[0]["function"]["name"].as_str().unwrap_or("");
    assert!(!name.contains('.'), "wire names are dot-free: {name}");
    assert_eq!(name, "probe__read");
    assert!(
        tcs[0]["function"]["arguments"].is_string(),
        "arguments is a JSON string"
    );
    let id = tcs[0]["id"].as_str().unwrap_or("");
    assert!(!id.is_empty(), "tool_call id present");
    let toolmsg = &s2[2];
    assert_eq!(toolmsg["role"], "tool");
    assert_eq!(
        toolmsg["tool_call_id"].as_str().unwrap_or(""),
        id,
        "tool result pairs with the call id"
    );
    assert!(
        toolmsg["content"]
            .as_str()
            .unwrap_or("")
            .contains("MARKER-777"),
        "tool result content is the real output: {}",
        toolmsg["content"]
    );

    // step 3: two pairs, in order
    let s3 = &steps[2];
    assert_eq!(s3.len(), 6, "mission + 2 pairs + tail: {s3:?}");

    // the KV-cache contract: step N+1s array is step Ns array minus its
    // state tail, plus the new pair, plus a fresh tail - the cached prefix
    // grows monotonically and no prior message is ever rewritten
    assert_eq!(
        &s3[..s2.len() - 1],
        &s2[..s2.len() - 1],
        "history prefix must be byte-identical across steps (append-only)"
    );
    assert_eq!(&s2[..1], &s1[..1], "the mission message never mutates");

    // no hand-rendered transcript markup survives anywhere
    let flat = ev
        .iter()
        .filter(|(k, _)| *k == EventKind::ModelCall)
        .map(|(_, p)| p.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !flat.contains("TRANSCRIPT (earlier tool calls):"),
        "no transcript heading in the native world"
    );
    assert!(
        !flat.contains("- probe.read("),
        "no hand-rendered transcript lines in the native world"
    );
}

/// assemble_messages: the log-sourced history becomes native message pairs
/// under the token budget, with compaction of the oldest into a handoff
/// message when over budget.
#[test]
fn assemble_messages_pairs_and_compaction() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let answer = log.path().join("work").join("task-0").join("answer.txt");
    let script = dir.path().join("script.jsonl");
    std::fs::write(
        &script,
        format!(
            "{{\"tool\":\"probe.read\",\"args\":{{\"path\":\"x\"}}}}\n{{\"tool\":\"probe.read\",\"args\":{{\"path\":\"y\"}}}}\n{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-0-SECRET\"}}}}",
            answer.display()
        ),
    )
    .unwrap();
    std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
    let config = dir.path().join("hairspring.toml");
    std::fs::write(
        &config,
        format!(
            r#"
[[tools]]
name = "answer.write"
command = ["{ANSWER}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{CHECKER}"]
subjects = ["*"]

[[tools]]
name = "probe.read"
command = ["{PROBE_BIN}"]
subjects = ["*"]

[[models]]
name = "scripted"
command = ["{SCRIPTED}"]
default = true
"#
        ),
    )
    .unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 10).unwrap();
    let r = l.run_mission("task-0").unwrap();
    assert!(r.passed);

    let reader = hs_log::StreamReader::open(log.path(), r.stream_id).unwrap();
    let events = reader.events().unwrap();

    // everything fits: one pair per operator ToolCall event, seq order
    let asm = assembler::assemble_messages(&reader, &events, 1_000_000);
    assert!(asm.compressed.is_none(), "no compaction under budget");
    assert_eq!(
        asm.messages.len(),
        6,
        "3 exchanges = 3 pairs: {:?}",
        asm.messages
    );
    assert_eq!(asm.messages[0]["role"], "assistant");
    assert_eq!(asm.messages[1]["role"], "tool");
    assert_eq!(
        asm.messages[0]["tool_calls"][0]["id"], asm.messages[1]["tool_call_id"],
        "pair ids match"
    );

    // tiny budget: oldest compact into a handoff message, newest pair verbatim
    let asm2 = assembler::assemble_messages(&reader, &events, 10);
    let c = asm2.compressed.as_ref().expect("over budget compacts");
    assert!(c.count >= 1, "at least the oldest exchange compacted");
    assert_eq!(asm2.messages[0]["role"], "user");
    assert!(
        asm2.messages[0]["content"]
            .as_str()
            .unwrap_or("")
            .starts_with("COMPACTED "),
        "compaction is a user handoff message: {}",
        asm2.messages[0]["content"]
    );
}

/// The verifier verdict is a native tool call: the request carries the
/// verdict.submit schema and the verdict arrives as tool args, not as
/// text-JSON parsed out of prose.
#[test]
fn verifier_verdict_is_a_native_tool_call() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("code.txt"), "broken\n").unwrap();
    for args in [
        &["init", "-q"][..],
        &["add", "."][..],
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "init",
        ][..],
    ] {
        assert!(std::process::Command::new("git")
            .args(args)
            .current_dir(&ws)
            .status()
            .unwrap()
            .success());
    }
    std::env::set_var("HS_SWE_WORKSPACE", &ws);
    let answer = log.path().join("work").join("task-21").join("answer.txt");
    let diff = "```diff\\n--- a/code.txt\\n+++ b/code.txt\\n@@ -1 +1 @@\\n-broken\\n+fixed\\n```";
    let script = dir.path().join("script.jsonl");
    std::fs::write(&script, format!(
        "{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"cat code.txt\",\"diff\":\"{diff}\"}}}}\n{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-21-SECRET\"}}}}",
        answer.display())).unwrap();
    std::env::set_var("HS_VF_SCRIPT", &script);
    let config = dir.path().join("hairspring.toml");
    std::fs::write(
        &config,
        format!(
            r#"
[[tools]]
name = "answer.write"
command = ["{ANSWER}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{CHECKER}"]
subjects = ["*"]

[[tools]]
name = "repo.exec"
command = ["{REPOEXEC}"]
subjects = ["*"]

[[models]]
name = "vfmodel"
command = ["{VFMODEL}"]
default = true
"#
        ),
    )
    .unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 5).unwrap();
    let r = l.run_mission("task-21").unwrap();
    assert!(r.passed, "honest verified work passes: {r:?}");
    let ev = events_of(log.path(), r.stream_id);
    let vcalls: Vec<&(EventKind, String)> = ev
        .iter()
        .filter(|(k, p)| *k == EventKind::ModelCall && p.contains("ADVERSARIAL VERIFIER"))
        .collect();
    assert_eq!(vcalls.len(), 1, "one verifier round: {}", vcalls.len());
    let payload: serde_json::Value = serde_json::from_str(&vcalls[0].1).unwrap();
    let tools = payload["tools"]
        .as_array()
        .expect("verifier call carries native tools");
    assert!(
        tools
            .iter()
            .any(|t| t["function"]["name"] == "verdict.submit"),
        "the verdict.submit schema is delivered: {tools:?}"
    );
    let completion: serde_json::Value =
        serde_json::from_str(payload["completion"].as_str().unwrap()).unwrap();
    assert_eq!(
        completion["tool"], "verdict.submit",
        "verdict arrives as a tool call: {completion}"
    );
    assert_eq!(
        completion["args"]["refuted"], false,
        "clean verdict in args: {completion}"
    );
    assert!(
        ev.iter()
            .any(|(k, p)| *k == EventKind::Feedback && p.contains("not_refuted")),
        "not_refuted verdict booked"
    );
}

/// A prose/text-JSON verdict (the old contract) is a verifier ERROR, never a
/// parsed verdict: no hand-rolled extraction survives.
#[test]
fn verifier_prose_reply_is_an_error_not_a_verdict() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let answer = log.path().join("work").join("task-22").join("answer.txt");
    // the "model" answers the mission AND would answer the verifier with
    // OLD-SHAPE bare text JSON - the loop must not honor it
    let script = dir.path().join("script.jsonl");
    std::fs::write(&script, format!(
        "{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-22-SECRET\"}}}}\n{{\"refuted\":true,\"findings\":[{{\"kind\":\"bug\",\"location\":\"answer\",\"detail\":\"prose verdict from the old contract\"}}],\"blocking\":\"none\"}}",
        answer.display())).unwrap();
    std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
    let config = dir.path().join("hairspring.toml");
    std::fs::write(
        &config,
        format!(
            r#"
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
command = ["{SCRIPTED}"]
default = true
"#
        ),
    )
    .unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 4).unwrap();
    let r = l.run_mission("task-22").unwrap();
    assert!(
        r.passed,
        "a malformed verdict never blocks the mission: {r:?}"
    );
    assert_eq!(r.steps, 1, "no refuted round burns steps: {r:?}");
    let ev = events_of(log.path(), r.stream_id);
    assert!(
        ev.iter()
            .any(|(k, p)| *k == EventKind::Feedback && p.contains("verifier_error")),
        "prose verdict books verifier_error: {ev:?}"
    );
    assert!(
        !ev.iter()
            .any(|(k, p)| *k == EventKind::Feedback && p.contains("\"verdict\": \"refuted\"")),
        "a prose verdict is never honored as refuted"
    );
}

/// realmodel::build_body_messages: the messages array passes through
/// verbatim behind the system message; tools are wire-mapped; the provider
/// enforces one tool call per reply.
#[test]
fn build_body_messages_shape() {
    let tools = serde_json::json!([{"type":"function","function":{"name":"repo.search","description":"d","parameters":{"type":"object","properties":{}}}}]);
    let messages = serde_json::json!([
        {"role":"user","content":"MISSION: task-x"},
        {"role":"assistant","tool_calls":[{"id":"call_3","type":"function","function":{"name":"repo__search","arguments":"{\"pattern\":\"foo\"}"}}]},
        {"role":"tool","tool_call_id":"call_3","content":"src/foo.rs:12: foo"},
        {"role":"user","content":"ATTEMPT: step 2 of 10"}
    ]);
    let body =
        realmodel::build_body_messages("kimi-k3", "sys", &messages, Some(&tools), None, true);
    assert_eq!(
        body["messages"].as_array().unwrap().len(),
        5,
        "system + 4 messages"
    );
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(
        body["messages"][1]["content"], "MISSION: task-x",
        "verbatim pass-through"
    );
    assert_eq!(body["messages"][2]["tool_calls"][0]["id"], "call_3");
    assert_eq!(body["tool_choice"], "required");
    assert_eq!(
        body["tools"][0]["function"]["name"], "repo__search",
        "wire-mapped names"
    );
}

/// cached_tokens pass-through (cache-win observability): the provider's
/// prompt-cache hit count must reach the operator ModelCall payload, so the
/// A-vs-B comparison can quantify the KV-cache win from the logs.
#[test]
fn cached_tokens_reach_the_model_call_payload() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let answer = log.path().join("work").join("task-0").join("answer.txt");
    let script = dir.path().join("script.jsonl");
    std::fs::write(
        &script,
        format!(
            "{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-0-SECRET\"}}}}",
            answer.display()
        ),
    )
    .unwrap();
    std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
    let config = dir.path().join("hairspring.toml");
    std::fs::write(
        &config,
        format!(
            r#"
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
command = ["{SCRIPTED}"]
default = true
"#
        ),
    )
    .unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 4).unwrap();
    let r = l.run_mission("task-0").unwrap();
    assert!(r.passed);
    let ev = events_of(log.path(), r.stream_id);
    let op = ev
        .iter()
        .filter(|(k, p)| *k == EventKind::ModelCall && p.contains("\"messages\""))
        .map(|(_, p)| serde_json::from_str::<serde_json::Value>(p).unwrap())
        .collect::<Vec<_>>();
    assert!(!op.is_empty(), "operator calls recorded");
    for v in &op {
        assert!(
            v["cached_tokens"].is_u64(),
            "operator payload carries cached_tokens: {v}"
        );
    }
    // the scripted fixture emits cached_tokens=42: the value is the
    // plugin's, not a kernel-computed estimate
    assert_eq!(op[0]["cached_tokens"], 42, "pass-through, not recomputed");
}
