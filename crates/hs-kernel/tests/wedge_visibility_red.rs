//! RED (2026-09-07): a wedged plugin call must be VISIBLE while it is wedged.
//!
//! Live evidence (conan-17302, 2026-09-07 07:18-07:36 UTC): a mission went
//! silent for 19 min - main thread in `futex_wait` inside the 1800s plugin
//! lease, plugin stderr wired to /dev/null, and the stream only records
//! COMPLETED calls, so there was no record of which plugin was even in
//! flight. An operator cannot distinguish "lease countdown running" from
//! "deadlock" without (a) a dispatch record written BEFORE the call is
//! awaited and (b) the plugin's stderr captured somewhere readable.

use std::io::Read;

const FIXTURE: &str = env!("CARGO_BIN_EXE_hs-fixture-plugin");

fn write_config(dir: &tempfile::TempDir, body: &str) -> std::path::PathBuf {
    let p = dir.path().join("hairspring.toml");
    std::fs::write(&p, body).unwrap();
    p
}

/// Concatenate every byte under `log_root` (stream segments are binary-framed
/// with inline JSON payloads - greppable).
fn slurp(dir: &std::path::Path) -> String {
    let mut out = String::new();
    for e in walk(dir) {
        if e.is_file() {
            let mut buf = Vec::new();
            if std::fs::File::open(&e)
                .and_then(|mut f| f.read_to_end(&mut buf))
                .is_ok()
            {
                out.push_str(&String::from_utf8_lossy(&buf));
            }
        }
    }
    out
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                v.extend(walk(&p));
            } else {
                v.push(p);
            }
        }
    }
    v
}

/// A call stuck inside its lease must already have a dispatch record in the
/// stream naming the plugin - before any response or error arrives.
#[test]
fn wedged_call_is_logged_in_flight_before_lease_expiry() {
    let dir = tempfile::tempdir().unwrap();
    let log_root = dir.path().join("logs");
    let cfg = write_config(
        &dir,
        &format!(
            r#"
[[tools]]
name = "sleeper"
command = ["{FIXTURE}", "hang-tool"]
subjects = ["*"]
lease_secs = 8
"#
        ),
    );
    let k = hs_kernel::Kernel::load_with_log(&cfg, &log_root).unwrap();
    let t = std::thread::spawn(move || {
        let _ = k.call_tool("anyone", "sleeper", serde_json::json!({}));
    });
    let mut found = false;
    for _ in 0..40 {
        let text = slurp(&log_root);
        if text.contains("\"stage\":\"dispatch\"") && text.contains("sleeper") {
            found = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    t.join().unwrap(); // lease fires at 8s; the call errors out
    assert!(
        found,
        "wedged call left no in-flight dispatch record in the stream"
    );
}

/// Plugin stderr must not vanish into /dev/null: when the kernel logs to a
/// `log_root`, each spawned plugin's stderr lands in a file under it.
#[test]
fn plugin_stderr_is_captured_under_log_root() {
    let dir = tempfile::tempdir().unwrap();
    let log_root = dir.path().join("logs");
    let cfg = write_config(
        &dir,
        &format!(
            r#"
[[tools]]
name = "spew"
command = ["{FIXTURE}", "stderr-spew"]
subjects = ["*"]
"#
        ),
    );
    let k = hs_kernel::Kernel::load_with_log(&cfg, &log_root).unwrap();
    let out = k
        .call_tool("anyone", "spew", serde_json::json!({}))
        .unwrap();
    assert_eq!(out.output["output"], "spew-ok");
    let text = slurp(&log_root.join("stderr"));
    assert!(
        text.contains("fixture-stderr-marker"),
        "plugin stderr not captured under log_root/stderr"
    );
}
