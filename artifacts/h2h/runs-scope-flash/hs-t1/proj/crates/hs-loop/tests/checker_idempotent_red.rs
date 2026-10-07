//! RED contract tests for checker.run idempotence (batch-3 trace autopsies,
//! Eric's 2026-09-05 trace-review directive).
//!
//! Proven defects under test (hs-plugin-swecheck judges the answer in the
//! LIVE mission workspace):
//! - 3862: a checker-green answer left the patch applied in the tree; when
//!   the model re-submitted the identical answer (duplicate guardrail fired,
//!   loop re-judged anyway), the second apply failed "patch does not apply"
//!   and the verdict flipped green->red - no verifier round could fire and
//!   the mission wedged until the wall. Same answer must judge the same.
//! - 8609: the post-failure reset is `git checkout -- .` only, so untracked
//!   files a failing patch created survive and poison the NEXT apply
//!   ("already exists in working directory").
//!
//! Contract: every checker.run judges from the committed base state - the
//! same answer file yields the same verdict on every call, and a failed
//! attempt leaves zero residue (tracked or untracked).

use std::io::{BufRead, BufReader, Write};
use std::path::Path;

const SWECHECK: &str = env!("CARGO_BIN_EXE_hs-plugin-swecheck");

/// Minimal git workspace: app.py committed at base, f2p "test" is a grep.
fn mk_ws() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::write(ws.join("app.py"), "x = 1\n").unwrap();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(ws)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap()
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "base"]);
    dir
}

struct Checker {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl Checker {
    fn spawn(ws: &Path, f2p: &str) -> Self {
        let mut child = std::process::Command::new(SWECHECK)
            .env("HS_SWE_WORKSPACE", ws)
            .env("HS_SWE_F2P", f2p)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("spawn swecheck");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Checker {
            child,
            stdin,
            stdout,
            next_id: 0,
        }
    }

    fn judge(&mut self, answer_path: &Path) -> serde_json::Value {
        self.next_id += 1;
        let call = serde_json::json!({
            "id": self.next_id,
            "method": "tool.call",
            "params": {"args": {"path": answer_path.display().to_string()}},
        });
        writeln!(self.stdin, "{call}").unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let resp: serde_json::Value = serde_json::from_str(&line).unwrap();
        resp["result"].clone()
    }
}

impl Drop for Checker {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn write_answer(dir: &Path, name: &str, diff_body: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, format!("```diff\n{diff_body}\n```\n")).unwrap();
    p
}

const PATCH_X2: &str =
    "diff --git a/app.py b/app.py\n--- a/app.py\n+++ b/app.py\n@@ -1 +1 @@\n-x = 1\n+x = 2";

/// 3862: re-judging the SAME checker-green answer must stay green.
#[test]
fn rejudge_same_answer_stays_green() {
    let ws = mk_ws();
    let answers = tempfile::tempdir().unwrap(); // production: answer lives in log/work, outside ws
    let ans = write_answer(answers.path(), "answer.txt", PATCH_X2);
    let mut c = Checker::spawn(ws.path(), "grep -q 'x = 2' app.py");
    let first = c.judge(&ans);
    assert_eq!(first["passed"], true, "first judge: {first}");
    let second = c.judge(&ans);
    assert_eq!(
        second["passed"], true,
        "same answer, second judge must stay green (3862 wedge): {second}"
    );
    // and the tree must still hold the applied patch (repo.exec flow
    // depends on the green state lingering)
    let body = std::fs::read_to_string(ws.path().join("app.py")).unwrap();
    assert!(body.contains("x = 2"), "green state must linger: {body}");
}

/// 8609: a failing patch that creates an untracked file must leave no
/// residue; the corrected re-submission must judge on its merits.
#[test]
fn failed_attempt_leaves_no_untracked_residue() {
    let ws = mk_ws();
    // v1: creates notes.txt but leaves app.py red for the f2p grep
    let answers = tempfile::tempdir().unwrap();
    let bad = write_answer(
        answers.path(),
        "a1.txt",
        "diff --git a/notes.txt b/notes.txt\nnew file mode 100644\n--- /dev/null\n+++ b/notes.txt\n@@ -0,0 +1 @@\n+hello",
    );
    let mut c = Checker::spawn(ws.path(), "grep -q 'x = 2' app.py");
    let first = c.judge(&bad);
    assert_eq!(first["passed"], false, "f2p red expected: {first}");
    // v2: same new file PLUS the app.py fix - must apply cleanly
    let good = write_answer(
        answers.path(),
        "a2.txt",
        "diff --git a/app.py b/app.py\n--- a/app.py\n+++ b/app.py\n@@ -1 +1 @@\n-x = 1\n+x = 2\ndiff --git a/notes.txt b/notes.txt\nnew file mode 100644\n--- /dev/null\n+++ b/notes.txt\n@@ -0,0 +1 @@\n+hello",
    );
    let second = c.judge(&good);
    assert_eq!(
        second["passed"], true,
        "residue from the failed attempt must not poison the retry (8609): {second}"
    );
}
