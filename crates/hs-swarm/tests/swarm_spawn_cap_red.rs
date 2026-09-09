//! RED: a LOST child's registry marker must not occupy a delegation
//! concurrency slot forever. Children are threads of their OWNER
//! plugin process; when the owner dies, its markers (<id>.spawn.json
//! with no <id>.report.json) outlive it. After the owner-proof fix,
//! poll's "lost" verdict and the spawn-side cap are driven by the SAME
//! predicate (owner alive with matching /proc start tick) - a dead
//! owner's marker frees its slot, a live owner's marker holds it.

use std::io::Write;
use std::process::{Command, Stdio};

const CID: &str = "00000000-0000-0000-0000-000000000042";
const PID: &str = "00000000-0000-0000-0000-000000000001";

fn spawn_once(dir: &std::path::Path) -> String {
    let mut p = Command::new("/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-swarm")
        .env("HS_SWARM_LOG_ROOT", dir)
        .env("HS_SWARM_MAX_CHILDREN", "1")
        .env("HS_SWARM_CONFIG", dir.join("hairspring.toml"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = p.stdin.take().unwrap();
    writeln!(
        stdin,
        "{{\"id\":1,\"method\":\"tool.call\",\"params\":{{\"args\":{{\"mission\":\"probe\",\"parent_stream\":\"{PID}\",\"child_stream_id\":\"{CID}\",\"depth\":0}}}}}}"
    )
    .unwrap();
    drop(stdin);
    let out = p.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn proc_start_ticks(pid: u32) -> u64 {
    // /proc/<pid>/stat field 22 (starttime, ticks since boot); comm may
    // contain parens/spaces, so anchor on the last ')'.
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    stat.rsplit(") ")
        .next()
        .unwrap()
        .split_whitespace()
        .nth(19)
        .unwrap()
        .parse()
        .unwrap()
}

fn owner_marker(dir: &std::path::Path, cid: &str, pid: u32, ticks: u64) {
    let swarm = dir.join("swarm");
    std::fs::create_dir_all(&swarm).unwrap();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    std::fs::write(
        swarm.join(format!("{cid}.spawn.json")),
        format!(
            "{{\"child_stream_id\":\"{cid}\",\"parent_stream\":\"p\",\"mission\":\"m\",\"started_at_ms\":{now_ms},\"owner_pid\":{pid},\"owner_start_ticks\":{ticks}}}"
        ),
    )
    .unwrap();
}

#[test]
fn r1_dead_owners_marker_does_not_consume_a_concurrency_slot() {
    let dir = std::env::temp_dir().join("swarm-cap-r1");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Minimal kernel config so the call reaches the cap check.
    std::fs::write(dir.join("hairspring.toml"), "[[models]]\nname = \"none\"\n").unwrap();
    // A real process stands in for the owning plugin; kill it so the
    // marker is a corpse (owner dead). Cap = 1 and this is the only
    // marker.
    let mut owner = Command::new("sleep")
        .arg("30")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let ticks = proc_start_ticks(owner.id());
    let _ = owner.kill();
    let _ = owner.wait();
    owner_marker(&dir, "deadbeef-0000-0000-0000-00000000dead", owner.id(), ticks);
    let body = spawn_once(&dir);
    assert!(
        !body.contains("concurrency limit"),
        "a dead owner's marker must not hold the only slot: {body}"
    );
    assert!(
        body.contains("child_stream_id") || body.contains("spawn:"),
        "call must reach spawn mechanics, got: {body}"
    );
}

#[test]
fn r2_live_owners_marker_still_refuses_at_cap() {
    let dir = std::env::temp_dir().join("swarm-cap-r2");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("hairspring.toml"), "[[models]]\nname = \"none\"\n").unwrap();
    // A LIVE owner: exactly what a marker for a child whose owner
    // process is running looks like. It must hold its slot.
    let mut owner = Command::new("sleep")
        .arg("30")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let ticks = proc_start_ticks(owner.id());
    owner_marker(&dir, "11111111-1111-4111-8111-111111111111", owner.id(), ticks);
    let body = spawn_once(&dir);
    let _ = owner.kill();
    let _ = owner.wait();
    assert!(
        body.contains("concurrency limit"),
        "a child with a live owner must hold its slot: {body}"
    );
}
