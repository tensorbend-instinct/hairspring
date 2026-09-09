//! hs-demo: a deterministic, killable executor standing in for the policy
//! layer at gate 1. It is stateless between steps except what it reads from
//! the log (spec section 2: "killable at any time").
//!
//! Workload: `state_0` = sha256("hs-demo-seed:<seed>"); `state_i` =
//! sha256(state_{i-1} || i). Each step appends a `ToolCall` event carrying
//! [step u64 LE][state 32B]; every 10th step also appends a Decision
//! breakpoint event with the same body; every 7th step appends an
//! Observation with an 8192-byte deterministic payload (exercises the
//! payload-by-hash blob store). Completion appends `GoalUpdate` with the final
//! state. Prints "FINAL <hex>".

use hs_core::{EventBuilder, EventKind, Payload, Event};
use hs_log::{StreamWriter, StreamReader};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::time::Duration;
use uuid::Uuid;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let get = |flag: &str| -> Option<String> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let dir = PathBuf::from(get("--dir").expect("--dir required"));
    let steps: u64 = get("--steps")
        .expect("--steps required")
        .parse()
        .expect("--steps must be an integer");
    let seed: u64 = get("--seed")
        .expect("--seed required")
        .parse()
        .expect("--seed must be an integer");
    let mode = args
        .iter()
        .find(|a| *a == "run" || *a == "resume")
        .expect("run|resume required");
    let delay_ms: u64 = get("--step-delay-ms")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let t0 = std::time::Instant::now();
    let (mut writer, mut step_done, mut state, resumed) = match mode.as_str() {
        "run" => {
            let stream = Uuid::new_v4();
            let w = StreamWriter::create(&dir, stream).expect("create stream");
            let s0: [u8; 32] = Sha256::digest(format!("hs-demo-seed:{seed}").as_bytes()).into();
            (w, 0u64, s0, None)
        }
        "resume" => {
            let stream = only_stream(&dir);
            let outcome = StreamWriter::resume(&dir, stream).expect("resume stream");
            eprintln!(
                "RESUMED stream={} recovered_events={} truncated_bytes={} setup_ms={}",
                stream,
                outcome.events_recovered,
                outcome.truncated_bytes,
                t0.elapsed().as_millis()
            );
            let reader = StreamReader::open(&dir, stream).expect("reopen stream we just wrote");
            let events = reader.events().expect("read stream we just wrote");
            if let Some((i, s)) = last_step(&events) { (outcome.writer, i, s, Some(outcome.events_recovered)) } else {
                let s0: [u8; 32] =
                    Sha256::digest(format!("hs-demo-seed:{seed}").as_bytes()).into();
                (outcome.writer, 0, s0, Some(outcome.events_recovered))
            }
        }
        _ => unreachable!(),
    };
    if resumed.is_some() {
        eprintln!("RESUME_FROM_STEP {}", step_done + 1);
    }

    let mut parent = writer.last_event_id();
    while step_done < steps {
        let next = step_done + 1;
        let mut h = Sha256::new();
        h.update(state);
        h.update(next.to_le_bytes());
        state = h.finalize().into();

        let mut body = Vec::with_capacity(40);
        body.extend(next.to_le_bytes());
        body.extend(state);
        let e = writer
            .append(EventBuilder::new(EventKind::ToolCall).payload(Payload::Inline(body.clone())))
            .expect("append step");
        parent = Some(e.event_id);

        if next % 10 == 0 {
            let e = writer
                .append(
                    EventBuilder::new(EventKind::Decision)
                        .payload(Payload::Inline(body.clone()))
                        .parent(parent.expect("assigned one line above")),
                )
                .expect("append breakpoint");
            parent = Some(e.event_id);
        }
        if next % 7 == 0 {
            let blob = expand_blob(&state, 8192);
            let e = writer
                .append(
                    EventBuilder::new(EventKind::Observation)
                        .payload(Payload::Inline(blob))
                        .parent(parent.expect("assigned one line above")),
                )
                .expect("append blob observation");
            parent = Some(e.event_id);
        }
        step_done = next;
        if delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(delay_ms));
        }
    }

    let mut final_body = Vec::with_capacity(8 + 32);
    final_body.extend(steps.to_le_bytes());
    final_body.extend(state);
    writer
        .append(
            EventBuilder::new(EventKind::GoalUpdate)
                .payload(Payload::Inline(final_body))
                .parent(parent.expect("assigned one line above")),
        )
        .expect("append goal_update");
    println!(
        "FINAL {}",
        state.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
}

fn only_stream(dir: &std::path::Path) -> Uuid {
    let entries: Vec<_> = std::fs::read_dir(dir.join("streams"))
        .expect("no streams dir: nothing to resume")
        .map(|e| {
            e.expect("readable streams dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert!(
        entries.len() == 1,
        "expected exactly one stream, found {}",
        entries.len()
    );
    Uuid::parse_str(entries.last().expect("asserted one stream"))
        .expect("stream dir name is a Uuid")
}

fn last_step(events: &[Event]) -> Option<(u64, [u8; 32])> {
    events.iter().rev().find_map(|e| {
        if e.kind != EventKind::ToolCall {
            return None;
        }
        let Payload::Inline(b) = &e.payload else {
            return None;
        };
        if b.len() < 40 {
            return None;
        }
        Some((
            u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
            <[u8; 32]>::try_from(&b[8..40]).expect("slice is exactly 32 bytes"),
        ))
    })
}

/// Deterministic 8KB body derived from state: repeated sha256 chaining.
fn expand_blob(state: &[u8; 32], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut cur = *state;
    while out.len() < len {
        cur = Sha256::digest(cur).into();
        out.extend_from_slice(&cur);
    }
    out.truncate(len);
    out
}
