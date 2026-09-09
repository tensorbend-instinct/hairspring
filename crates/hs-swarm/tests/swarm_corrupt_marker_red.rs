//! RED: a partition-corrupt registry marker must be reported LOST, never
//! "running". Swarm registry markers are plain .spawn.json files; if the
//! plugin process is killed mid-write (crash storm), the marker can be
//! truncated. Read chain today: `read_to_string` ok -> `serde_json` parse Err
//! -> None -> `is_some_and(false)` -> stale=false -> poll answers "running"
//! FOREVER on a child that either never started or died, and
//! `running_count` bills it a concurrency slot for the rest of time.
//! Honesty rule already established by "lost": an undecidable marker is a
//! DEAD child, never a live one.

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
    drop(stdin);
    let out = p.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn r1_corrupt_marker_reports_lost_not_running() {
    let dir = std::env::temp_dir().join("swarm-corrupt-r1");
    let _ = std::fs::remove_dir_all(&dir);
    let swarm = dir.join("swarm");
    std::fs::create_dir_all(&swarm).unwrap();
    let cid = "00000000-0000-0000-0000-0000000000c1";
    // Truncated as killed mid-write: valid prefix, broken JSON.
    std::fs::write(
        swarm.join(format!("{cid}.spawn.json")),
        "{\"child_stream_id\":\"00000000-0000-0000-0000-0000000000c1\",\"parent_stream\":\"p\",\"mission\":\"m\",\"started_at_",
    )
    .unwrap();
    let body = poll_once(&dir, cid);
    assert!(
        body.contains("\"lost\""),
        "a corrupt marker must report lost, got: {body}"
    );
    assert!(
        !body.contains("\"running\""),
        "never report an undecidable marker as running: {body}"
    );
}

#[test]
fn r2_corrupt_marker_does_not_consume_a_concurrency_slot() {
    let dir = std::env::temp_dir().join("swarm-corrupt-r2");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("hairspring.toml"), "[[models]]\nname = \"none\"\n").unwrap();
    let swarm = dir.join("swarm");
    std::fs::create_dir_all(&swarm).unwrap();
    std::fs::write(
        swarm.join("deadbeef-0000-0000-0000-0000000000d2.spawn.json"),
        "{\"child_stream_id\":\"deadbeef-0000-0000-0000-0000000000d2\",\"parent_",
    )
    .unwrap();
    let mut p = Command::new("/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-swarm")
        .env("HS_SWARM_LOG_ROOT", &dir)
        .env("HS_SWARM_MAX_CHILDREN", "1")
        .env("HS_SWARM_CONFIG", dir.join("hairspring.toml"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = p.stdin.take().unwrap();
    writeln!(
        stdin,
        "{{\"id\":1,\"method\":\"tool.call\",\"params\":{{\"args\":{{\"mission\":\"probe\",\"parent_stream\":\"00000000-0000-0000-0000-000000000001\",\"child_stream_id\":\"00000000-0000-0000-0000-000000000042\",\"depth\":0}}}}}}"
    )
    .unwrap();
    drop(stdin);
    let out = p.wait_with_output().unwrap();
    let body = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        !body.contains("concurrency limit"),
        "a corrupt marker must not hold the only slot: {body}"
    );
}
