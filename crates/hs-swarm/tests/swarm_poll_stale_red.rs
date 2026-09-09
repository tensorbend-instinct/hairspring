//! RED: poll honesty for a marker the owning plugin died on. Children
//! are THREADS of the swarm plugin process; if the kernel respawns the
//! plugin, the new process's registry read finds <id>.spawn.json with
//! no <id>.report.json and (before the fix) answered "running"
//! forever - the parent burns steps polling a corpse. Fix: the marker
//! carries `started_at_ms`; a plugin that STARTED AFTER the marker stamp
//! cannot own the child, so poll reports "lost" (honest) instead.

use std::io::Write;
use std::process::{Command, Stdio};

fn poll_once(dir: &std::path::Path, cid: &str) -> String {
    let mut p = Command::new("/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-swarm")
        .args(["--as", "agent.spawn_poll"])
        .env("HS_SWARM_LOG_ROOT", dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = p.stdin.take().unwrap();
    writeln!(
        stdin,
        "{{\"id\":1,\"method\":\"tool.call\",\"params\":{{\"args\":{{\"child_stream_id\":\"{cid}\"}}}}}}"
    )
    .unwrap();
    drop(stdin); // EOF ends the serve loop
    let out = p.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn r1_stale_marker_from_a_dead_plugin_reports_lost_not_running() {
    let dir = std::env::temp_dir().join("swarm-poll-stale-r1");
    let _ = std::fs::remove_dir_all(&dir);
    let swarm = dir.join("swarm");
    std::fs::create_dir_all(&swarm).unwrap();
    let cid = "00000000-0000-0000-0000-000000000001";
    // Stamped far in the past: the polling plugin process starts NOW,
    // after the stamp, so it cannot own this child.
    std::fs::write(
        swarm.join(format!("{cid}.spawn.json")),
        format!(
            "{{\"child_stream_id\":\"{cid}\",\"parent_stream\":\"p\",\"mission\":\"m\",\"started_at_ms\":1000000}}"
        ),
    )
    .unwrap();
    let body = poll_once(&dir, cid);
    assert!(
        body.contains("\"lost\""),
        "a marker older than the polling process must report lost, got: {body}"
    );
    assert!(
        !body.contains("\"running\""),
        "never report a corpse as running: {body}"
    );
}

#[test]
fn r2_live_owner_marker_reports_running() {
    // The honest contract: a marker that names a verifiably ALIVE owner
    // process (pid + /proc start ticks) reports running from a reader in
    // ANY process. This test previously faked freshness with a FUTURE
    // started_at_ms - the only representation the old reader-lifetime
    // predicate could express. Owner-proof markers make the real
    // scenario testable. (Full live-owner cycle incl. dead-owner and
    // pid-reuse arms lives in spawn_registry_owner_red.rs.)
    let dir = std::env::temp_dir().join("swarm-poll-stale-r2");
    let _ = std::fs::remove_dir_all(&dir);
    let swarm = dir.join("swarm");
    std::fs::create_dir_all(&swarm).unwrap();
    let cid = "00000000-0000-0000-0000-000000000002";
    let mut owner = std::process::Command::new("sleep")
        .arg("30")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let stat = std::fs::read_to_string(format!("/proc/{}/stat", owner.id())).unwrap();
    let ticks: u64 = stat
        .rsplit(") ")
        .next()
        .unwrap()
        .split_whitespace()
        .nth(19)
        .unwrap()
        .parse()
        .unwrap();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    std::fs::write(
        swarm.join(format!("{cid}.spawn.json")),
        format!(
            "{{\"child_stream_id\":\"{cid}\",\"parent_stream\":\"p\",\"mission\":\"m\",\"started_at_ms\":{now_ms},\"owner_pid\":{},\"owner_start_ticks\":{ticks}}}",
            owner.id()
        ),
    )
    .unwrap();
    let body = poll_once(&dir, cid);
    let _ = owner.kill();
    let _ = owner.wait();
    assert!(
        body.contains("\"running\""),
        "a marker with a live owner reports running, got: {body}"
    );
}
