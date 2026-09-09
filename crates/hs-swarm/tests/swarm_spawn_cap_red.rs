//! RED: a LOST child's registry marker must not occupy a delegation
//! concurrency slot forever. Children are threads of the plugin
//! process; when the owning plugin dies, its markers
//! (<id>.spawn.json with no <id>.report.json) outlive it. Poll
//! already reports such markers "lost" (they predate the surviving
//! process), but the spawn-side cap check counted them as RUNNING:
//! after `max_children` plugin deaths every spawn was refused on a
//! completely empty pipeline. Fix: `running_count` applies the same
//! stale-vs-process-start predicate as poll.

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

fn plant_marker(dir: &std::path::Path, cid: &str, started_at_ms: i64) {
    let swarm = dir.join("swarm");
    std::fs::create_dir_all(&swarm).unwrap();
    std::fs::write(
        swarm.join(format!("{cid}.spawn.json")),
        format!(
            "{{\"child_stream_id\":\"{cid}\",\"parent_stream\":\"p\",\"mission\":\"m\",\"started_at_ms\":{started_at_ms}}}"
        ),
    )
    .unwrap();
}

#[test]
fn r1_stale_corpse_marker_does_not_consume_a_concurrency_slot() {
    let dir = std::env::temp_dir().join("swarm-cap-r1");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Minimal kernel config so the call reaches the cap check.
    std::fs::write(dir.join("hairspring.toml"), "[[models]]\nname = \"none\"\n").unwrap();
    // Corpse: stamped long before ANY process started now; poll reports
    // it "lost". Cap = 1 and this is the only marker.
    plant_marker(&dir, "deadbeef-0000-0000-0000-00000000dead", 1_000_000);
    let body = spawn_once(&dir);
    assert!(
        !body.contains("concurrency limit"),
        "a dead plugin's marker must not hold the only slot: {body}"
    );
    assert!(
        body.contains("child_stream_id") || body.contains("spawn:"),
        "call must reach spawn mechanics, got: {body}"
    );
}

#[test]
fn r2_fresh_marker_from_the_living_plugin_still_refuses_at_cap() {
    let dir = std::env::temp_dir().join("swarm-cap-r2");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("hairspring.toml"), "[[models]]\nname = \"none\"\n").unwrap();
    // Future stamp: exactly what a thread-of-this-process marker looks
    // like relative to the plugin's own start. It must still count.
    plant_marker(&dir, "11111111-1111-4111-8111-111111111111", i64::MAX / 2);
    let body = spawn_once(&dir);
    assert!(
        body.contains("concurrency limit"),
        "a genuinely running child must hold its slot: {body}"
    );
}
