//! GATE 2 ACCEPTANCE (spec section 10, row 2):
//!   Add a new tool and a new model by configuration only:
//!   zero harness code changes, zero redeploy.
//!
//! Falsifiable: if the driver process is restarted, or the harness binaries
//! are rebuilt between phases, or the new capabilities fail, the gate fails.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Driver {
    // held so the driver process stays alive for the whole test
    #[allow(dead_code)]
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    pid: u32,
}

impl Driver {
    fn start(dir: &std::path::Path, config: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_hs-gate2-driver"))
            .args([
                "--dir",
                dir.to_str().unwrap(),
                "--config",
                config.to_str().unwrap(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        let stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "READY", "driver did not start");
        Driver {
            child,
            stdin,
            stdout,
            pid,
        }
    }
    fn cmd(&mut self, c: &str) -> String {
        writeln!(self.stdin, "{c}").unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        line.trim().to_string()
    }
}

const ECHO: &str = env!("CARGO_BIN_EXE_hs-plugin-echo");
const UPPER: &str = env!("CARGO_BIN_EXE_hs-plugin-upper");
const MODEL1: &str = env!("CARGO_BIN_EXE_hs-plugin-fakemodel");
const MODEL2: &str = env!("CARGO_BIN_EXE_hs-plugin-fakemodel2");

#[test]
fn gate2_proof_add_tool_and_model_by_config_only() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("hairspring.toml");

    // Phase 1: harness runs with one tool, one model.
    std::fs::write(
        &config,
        format!(
            r#"
[[tools]]
name = "echo"
command = ["{ECHO}"]
subjects = ["*"]

[[models]]
name = "fake-v1"
command = ["{MODEL1}"]
default = true
"#
        ),
    )
    .unwrap();
    let mut d = Driver::start(dir.path(), &config);
    let pid_before = d.pid;

    assert!(d.cmd("tool echo hello").starts_with("OK echo:"));
    assert!(d.cmd("model fake-v1 prove me").starts_with("OK fake-v1:"));
    // the new capabilities do not exist yet
    assert!(d.cmd("tool upper hello").starts_with("ERR"));
    assert!(d.cmd("model fake-v2 hi").starts_with("ERR"));

    // THE CHANGE: config file only. Two plugin executables that the harness
    // has never seen are declared. Harness code untouched.
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(
        &config,
        format!(
            r#"
[[tools]]
name = "echo"
command = ["{ECHO}"]
subjects = ["*"]

[[tools]]
name = "upper"
command = ["{UPPER}"]
subjects = ["*"]

[[models]]
name = "fake-v1"
command = ["{MODEL1}"]
default = true

[[models]]
name = "fake-v2"
command = ["{MODEL2}"]
"#
        ),
    )
    .unwrap();

    assert_eq!(d.cmd("reload"), "RELOADED true");

    // Phase 2: new capabilities live, same process.
    assert_eq!(d.cmd("tool upper hello"), "OK upper: HELLO");
    assert!(d
        .cmd("model fake-v2 second model")
        .starts_with("OK fake-v2:"));
    assert!(d.cmd("tool echo still here").starts_with("OK echo:"));
    assert_eq!(
        d.pid, pid_before,
        "harness process was restarted: gate falsified"
    );

    // And every call - before and after the change - is on the canonical log.
    drop(d);
    let mut entries: Vec<_> = std::fs::read_dir(dir.path().join("streams"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(entries.len(), 1);
    let sid = uuid::Uuid::parse_str(&entries.pop().unwrap()).unwrap();
    let events = hs_log::StreamReader::open(dir.path(), sid)
        .unwrap()
        .events()
        .unwrap();
    let kinds: Vec<_> = events.iter().map(|e| e.kind).collect();
    assert!(
        kinds
            .iter()
            .filter(|k| **k == hs_core::EventKind::ToolCall)
            .count()
            >= 3
    );
    assert!(
        kinds
            .iter()
            .filter(|k| **k == hs_core::EventKind::ModelCall)
            .count()
            >= 2
    );
    let joined = events
        .iter()
        .map(|e| match &e.payload {
            hs_core::Payload::Inline(b) => String::from_utf8_lossy(b).into_owned(),
            other => format!("{:?}", other),
        })
        .collect::<String>();
    assert!(
        joined.contains("HELLO") && joined.contains("fake-v2"),
        "phase-2 calls not on the log"
    );
    hs_log::verify_stream(dir.path(), sid).unwrap();

    println!("PROOF-GATE2 add-by-config-only: PASS");
    println!("  same harness process (pid {pid_before}) before and after; new tool 'upper' and new model 'fake-v2' live after config reload");
    println!(
        "  {} events on the canonical log, chain verified",
        events.len()
    );
}
