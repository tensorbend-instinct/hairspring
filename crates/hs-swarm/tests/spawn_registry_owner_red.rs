//! RED: the stale/corpse predicate must name the actual OWNER of the
//! child thread. Children are threads of the agent.spawn plugin process;
//! `agent.spawn_poll` runs in a SEPARATE process (the kernel registers the
//! two names as two commands). RESTART ORDER decides nothing: the stamp
//! comparison only ever named a reader-relative guess.
//! Live-owner proof is marker-carried (pid + /proc start ticks + a
//! non-zombie state): any reader can verify it.. The pre-fix predicate compared the marker
//! stamp with the POLLING process's own start time: the moment the poll
//! process restarts mid-session (kernel respawn), EVERY live child reads
//! "lost", and the parent abandons a child whose thread is still burning
//! tokens; symmetrically, when spawn restarts but poll does not, DEAD
//! children read "running" forever (the exact bug the stale fix targeted
//! - it only held in the same-process case).

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

fn proc_start_ticks(pid: u32) -> u64 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    // starttime is field 22; comm (field 2) may contain spaces/parens, so
    // anchor on the last ')' - fields after it start at 3 (state).
    let after = stat.rsplit(") ").next().unwrap();
    after.split_whitespace().nth(19).unwrap().parse().unwrap()
}

fn write_owner_marker(swarm: &std::path::Path, cid: &str, owner_pid: u32) {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    std::fs::write(
        swarm.join(format!("{cid}.spawn.json")),
        format!(
            "{{\"child_stream_id\":\"{cid}\",\"parent_stream\":\"p\",\"mission\":\"m\",\"started_at_ms\":{now_ms},\"owner_pid\":{owner_pid},\"owner_start_ticks\":{}}}",
            proc_start_ticks(owner_pid)
        ),
    )
    .unwrap();
}

#[test]
fn u1_live_owner_reads_running_from_any_reader_process() {
    let dir = std::env::temp_dir().join("swarm-owner-u1");
    let _ = std::fs::remove_dir_all(&dir);
    let swarm = dir.join("swarm");
    std::fs::create_dir_all(&swarm).unwrap();
    let cid = "00000000-0000-0000-0000-000000000009";
    // A REAL living owner process stands in for the spawn plugin holding
    // child threads. The marker (including its ms stamp) is written
    // BEFORE the poll process starts - the old predicate reads it stale.
    let mut owner = Command::new("sleep")
        .arg("30")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    write_owner_marker(&swarm, cid, owner.id());
    let body = poll_once(&dir, cid);
    let _ = owner.kill();
    let _ = owner.wait();
    assert!(
        body.contains("\"running\""),
        "a child whose owner process is verifiably alive must read running, got: {body}"
    );
    assert!(
        !body.contains("\"lost\""),
        "owner alive but verdict lost: the poll process judged by ITS OWN start time: {body}"
    );
}

#[test]
fn u2_dead_owner_reads_lost_and_frees_the_slot() {
    let dir = std::env::temp_dir().join("swarm-owner-u2");
    let _ = std::fs::remove_dir_all(&dir);
    let swarm = dir.join("swarm");
    std::fs::create_dir_all(&swarm).unwrap();
    let cid = "00000000-0000-0000-0000-00000000000a";
    let mut owner = Command::new("sleep")
        .arg("30")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    write_owner_marker(&swarm, cid, owner.id());
    let _ = owner.kill();
    let _ = owner.wait();
    let body = poll_once(&dir, cid);
    assert!(
        body.contains("\"lost\""),
        "owner dead: the child thread died with it, verdict must be lost, got: {body}"
    );
}

#[test]
fn u4_zombie_owner_reads_lost_not_running() {
    // A killed plugin lingers as a ZOMBIE (its /proc entry, with its
    // original start tick, stays until the parent reaps it). The e2e
    // r3 proof (hs-loop swarm_async_red: pkill the plugin, parent joins
    // on a live-polling loop) failed wall_killed when the predicate
    // read a tick match on the zombie as "alive". The owner here is a
    // CHILD OF THIS TEST PROCESS, so dropping kill() without wait()
    // leaves exactly the zombie /proc state the kernel leaves behind.
    let dir = std::env::temp_dir().join("swarm-owner-u4");
    let _ = std::fs::remove_dir_all(&dir);
    let swarm = dir.join("swarm");
    std::fs::create_dir_all(&swarm).unwrap();
    let cid = "00000000-0000-0000-0000-00000000000c";
    let mut owner = Command::new("sleep")
        .arg("30")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    write_owner_marker(&swarm, cid, owner.id());
    let _ = owner.kill();
    // No wait(): the pid now maps to a zombie owned by THIS process.
    std::thread::sleep(std::time::Duration::from_millis(100));
    let body = poll_once(&dir, cid);
    let _ = owner.wait();
    assert!(
        body.contains("\"lost\""),
        "a zombie (unreaped) owner is DEAD - the child threads are gone: {body}"
    );
}

#[test]
fn u3_reused_pid_with_different_start_ticks_reads_lost() {
    let dir = std::env::temp_dir().join("swarm-owner-u3");
    let _ = std::fs::remove_dir_all(&dir);
    let swarm = dir.join("swarm");
    std::fs::create_dir_all(&swarm).unwrap();
    let cid = "00000000-0000-0000-0000-00000000000b";
    let owner = Command::new("sleep")
        .arg("30")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // Marker claims THIS pid but a start tick the real process does not
    // have: as it would look if pid reuse brought a fresh process into
    // the slot of a dead owner.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    std::fs::write(
        swarm.join(format!("{cid}.spawn.json")),
        format!(
            "{{\"child_stream_id\":\"{cid}\",\"parent_stream\":\"p\",\"mission\":\"m\",\"started_at_ms\":{now_ms},\"owner_pid\":{},\"owner_start_ticks\":1}}",
            owner.id()
        ),
    )
    .unwrap();
    let body = poll_once(&dir, cid);
    let mut owner = owner;
    let _ = owner.kill();
    let _ = owner.wait();
    assert!(
        body.contains("\"lost\""),
        "pid match but start-tick mismatch = different process = corpse: {body}"
    );
}
