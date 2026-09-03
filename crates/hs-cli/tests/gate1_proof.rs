//! GATE 1 ACCEPTANCE (spec section 10, row 1):
//!   1. Kill the executor mid-run; resume from breakpoint with zero state
//!      loss within the tier-A budget (< 5 s warm).
//!   2. Rewind to an arbitrary seq and replay to identical state.
//!   3. Corrupt a byte in the log; the hash chain catches it.
//!
//! Written before hs-demo existed. These are falsifiable: each test names
//! the exact condition that would falsify the gate.

use hs_core::*;
use hs_log::*;
use sha2::{Digest, Sha256};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const STEPS: u64 = 60;
const SEED: u64 = 42;

/// Independent oracle: the demo workload's state after `n` steps, recomputed
/// here from the workload definition (not from the binary under test).
fn oracle_state(seed: u64, n: u64) -> [u8; 32] {
    let mut s: [u8; 32] = Sha256::digest(format!("hs-demo-seed:{seed}").as_bytes()).into();
    for i in 1..=n {
        let mut h = Sha256::new();
        h.update(s);
        h.update(i.to_le_bytes());
        s = h.finalize().into();
    }
    s
}

fn run_demo(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    let mut full = vec![
        "--dir",
        dir.to_str().unwrap(),
        "--steps",
        "60",
        "--seed",
        "42",
    ];
    full.extend_from_slice(args);
    Command::new(env!("CARGO_BIN_EXE_hs-demo"))
        .args(&full)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

fn final_hash_of(out: &std::process::Output) -> String {
    let s = String::from_utf8_lossy(&out.stdout);
    s.lines()
        .find_map(|l| l.strip_prefix("FINAL "))
        .unwrap_or_else(|| {
            panic!(
                "no FINAL in: {s}\nstderr: {}",
                String::from_utf8_lossy(&out.stderr)
            )
        })
        .to_string()
}

fn stream_of(dir: &std::path::Path) -> uuid::Uuid {
    let mut entries: Vec<_> = std::fs::read_dir(dir.join("streams"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(entries.len(), 1, "expected exactly one stream");
    uuid::Uuid::parse_str(&entries.pop().unwrap()).unwrap()
}

#[test]
fn gate1_proof_1_kill_mid_run_resume_zero_loss() {
    // Reference: uninterrupted run.
    let refdir = tempfile::tempdir().unwrap();
    let out = run_demo(refdir.path(), &["run", "--step-delay-ms", "0"]);
    assert!(out.status.success());
    let ref_final = final_hash_of(&out);
    let ref_sid = stream_of(refdir.path());
    let ref_events = StreamReader::open(refdir.path(), ref_sid)
        .unwrap()
        .events()
        .unwrap();
    assert_eq!(
        ref_final,
        hex(oracle_state(SEED, STEPS)),
        "demo disagrees with oracle"
    );

    // Victim: same run, killed with SIGKILL partway through.
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_hs-demo"))
        .args([
            "--dir",
            dir.path().to_str().unwrap(),
            "--steps",
            "60",
            "--seed",
            "42",
            "run",
            "--step-delay-ms",
            "40",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    child.kill().unwrap(); // SIGKILL: no atexit, no flush, no mercy
    let status = child.wait().unwrap();
    assert!(!status.success());

    let sid = stream_of(dir.path());
    let pre_resume_events = StreamReader::open(dir.path(), sid)
        .unwrap()
        .events()
        .unwrap();
    assert!(
        pre_resume_events.len() < ref_events.len(),
        "victim finished before the kill: test invalid"
    );
    assert!(
        pre_resume_events.len() >= 5,
        "kill landed too early: test invalid"
    );
    let killed_at_seq = pre_resume_events.last().unwrap().seq;

    // Resume from the log. Tier-A budget: < 5 s warm.
    let t0 = Instant::now();
    let out = run_demo(dir.path(), &["resume", "--step-delay-ms", "0"]);
    let resume_wall = t0.elapsed();
    assert!(
        out.status.success(),
        "resume failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let resumed_final = final_hash_of(&out);

    // THE PROOF:
    // (a) identical final state - zero state loss, bit for bit
    assert_eq!(resumed_final, ref_final, "state diverged after kill+resume");
    assert_eq!(resumed_final, hex(oracle_state(SEED, STEPS)));
    // (b) history was not rewritten - every pre-kill event is byte-identical
    let post = StreamReader::open(dir.path(), sid)
        .unwrap()
        .events()
        .unwrap();
    assert_eq!(
        &post[..pre_resume_events.len()],
        &pre_resume_events[..],
        "resume rewrote history"
    );
    // (c) exactly the reference event count, contiguous seqs, no duplicates
    assert_eq!(post.len(), ref_events.len());
    for (i, e) in post.iter().enumerate() {
        assert_eq!(e.seq, i as u64, "seq gap/dup at {i}");
    }
    // (d) the chain verifies end to end
    verify_stream(dir.path(), sid).unwrap();
    // (e) tier-A budget
    assert!(
        resume_wall < Duration::from_secs(5),
        "resume took {:?}, over tier-A budget",
        resume_wall
    );

    println!("PROOF-1 kill-mid-run resume: PASS");
    println!(
        "  killed after seq {killed_at_seq} of {} events; resume wall time {:?} (< 5 s tier-A)",
        ref_events.len(),
        resume_wall
    );
    println!(
        "  final state {} == uninterrupted reference == oracle",
        resumed_final
    );
}

#[test]
fn gate1_proof_2_rewind_and_replay_to_identical_state() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_demo(dir.path(), &["run", "--step-delay-ms", "0"]);
    assert!(out.status.success());
    let sid = stream_of(dir.path());
    let reader = StreamReader::open(dir.path(), sid).unwrap();
    let events = reader.events().unwrap();

    // Every Decision event is a breakpoint carrying the state hash at that
    // step. Rewind to each one and check it against the oracle.
    let mut checked = 0;
    for e in &events {
        if e.kind != EventKind::Decision {
            continue;
        }
        let Payload::Inline(body) = &e.payload else {
            panic!("breakpoint payload not inline")
        };
        let step = u64::from_le_bytes(body[0..8].try_into().unwrap());
        let recorded: [u8; 32] = body[8..40].try_into().unwrap();
        // Rewind: replay only up to this seq, rebuild the state from the log.
        let prefix = reader.replay_to(e.seq).unwrap();
        let rebuilt = hs_demo_state_from_events(&prefix);
        assert_eq!(
            rebuilt,
            Some((step, recorded)),
            "replay to seq {} diverged",
            e.seq
        );
        assert_eq!(
            recorded,
            oracle_state(SEED, step),
            "breakpoint state wrong at step {step}"
        );
        checked += 1;
    }
    assert!(checked >= 5, "expected breakpoints to check, got {checked}");
    println!(
        "PROOF-2 rewind/replay: PASS - {checked} breakpoints replayed to oracle-identical state"
    );
}

/// Rebuild demo state purely from log events (the resume read path).
fn hs_demo_state_from_events(events: &[Event]) -> Option<(u64, [u8; 32])> {
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
            u64::from_le_bytes(b[0..8].try_into().unwrap()),
            b[8..40].try_into().unwrap(),
        ))
    })
}

#[test]
fn gate1_proof_3_corrupted_byte_is_caught() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_demo(dir.path(), &["run", "--step-delay-ms", "0"]);
    assert!(out.status.success());
    let sid = stream_of(dir.path());
    let victim_seq = 3u64;
    hs_log::testing::corrupt_event_byte(dir.path(), sid, victim_seq, 55);
    let err = verify_stream(dir.path(), sid).unwrap_err();
    assert_eq!(err.seq, victim_seq, "corruption reported at wrong seq");
    println!(
        "PROOF-3 corruption caught: PASS - flipped byte reported at seq {} ({:?})",
        err.seq, err.kind
    );
}

#[test]
fn gate1_determinism_two_runs_identical() {
    let f = |tmp: &std::path::Path| {
        let out = run_demo(tmp, &["run", "--step-delay-ms", "0"]);
        assert!(out.status.success());
        final_hash_of(&out)
    };
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    assert_eq!(f(a.path()), f(b.path()));
}

fn hex(b: [u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
