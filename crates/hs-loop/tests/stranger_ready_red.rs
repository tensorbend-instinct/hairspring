//! Stranger-readiness RED (2026-09-09, live stranger-path audit on a clean
//! clone of the repo, run exactly as the README/example config tell a
//! first-time user to run it):
//!
//! T1 (live burn, run dir /tmp/hs-demo): on the SHIPPED terminal surface
//! (`term.exec` + `answer.submit` + `checker.run`, `hairspring.example.toml`), a
//! mission can NEVER complete: hs-repl wires `HS_SELFCHECK_DIRECT` for the
//! live surface but never `HS_ANSWER_RAW`, so `answer.submit` takes the
//! candidate-worktree git path and answers "nothing to submit: make your
//! fix with edit.patch first" - a tool the config does not even register -
//! until `steps_exhausted`. Observed live: 25 model calls, `passed=false`,
//! the file the agent wrote sitting right there in the workdir.
//!
//! T2: `hs-repl --help` PANICKED (`--config required`, hs-repl.rs:35)
//! instead of printing usage. The first command a stranger runs must not
//! crash.
//!
//! T3 (live burns, stranger runs 1 AND 2): the shipped example config's
//! scripted model PANICKED at kernel startup when `HS_SEQMODEL_SCRIPT` was
//! unset (`called Result::unwrap() on an Err value: NotPresent`), killing
//! every stranger run - key or no key - with `plugin exited (EOF)`. An
//! inert-until-called offline model: describe answers, a call without a
//! script returns a `$error` naming the env var, the process stays alive.

use hs_loop::repl::{ReplSession, goal_slug};

const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");
const ANSWERSUBMIT: &str = env!("CARGO_BIN_EXE_hs-plugin-answersubmit");
const SELFCHECK: &str = env!("CARGO_BIN_EXE_hs-plugin-selfcheck");
const TERMEXEC: &str = env!("CARGO_BIN_EXE_hs-plugin-termexec");
const REPL: &str = env!("CARGO_BIN_EXE_hs-repl");

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn write(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p
}

/// The shipped example surface, reduced to the tools a one-shot mission
/// needs (verbatim tool set and model wiring from hairspring.example.toml).
fn live_config(dir: &std::path::Path) -> std::path::PathBuf {
    write(
        dir,
        "hairspring.toml",
        &format!(
            r#"
[[tools]]
name = "term.exec"
command = ["{TERMEXEC}"]
subjects = ["*"]

[[tools]]
name = "answer.submit"
command = ["{ANSWERSUBMIT}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{SELFCHECK}"]
subjects = ["*"]

[[models]]
name = "scripted"
command = ["{SCRIPTED}"]
default = true
subjects = ["*"]
"#
        ),
    )
}

#[test]
fn t1_live_surface_mission_completes() {
    let _g = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let goal = "write hello.txt containing hello";
    let answer = log
        .path()
        .join("work")
        .join(goal_slug(goal))
        .join("answer.txt");
    let line1 = "{\"tool\":\"term.exec\",\"args\":{\"command\":\"printf 'hello\\n' > hello.txt && mkdir -p .hs && printf 'grep -q hello hello.txt\\n' > .hs/checks\"}}";
    let line2 = format!(
        "{{\"tool\":\"answer.submit\",\"args\":{{\"path\":\"{}\",\"summary\":\"wrote hello.txt containing hello; verified by the declared grep check\"}}}}",
        answer.display()
    );
    let script = write(dir.path(), "script.jsonl", &format!("{line1}\n{line2}\n"));
    unsafe {
        std::env::remove_var("HS_MCP_SERVERS");
        std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
    }
    let mut session = ReplSession::load(&live_config(dir.path()), log.path(), false, 8)
        .expect("session load over the shipped terminal surface");
    unsafe {
        std::env::remove_var("HS_SEQMODEL_SCRIPT");
    }
    let r = session.run_goal(goal).expect("mission runs to a result");
    assert!(
        r.passed,
        "a live-surface mission that wrote the file and declared green \
         checks must COMPLETE (live burn 2026-09-09: answer.submit took the \
         candidate-worktree git path, 25 calls, steps_exhausted): {r:?}"
    );
    let written = std::fs::read_to_string(&answer).unwrap_or_default();
    assert!(
        written.contains("hello.txt"),
        "the submission is the agent's summary on the live surface: {written}"
    );
}

#[test]
fn t2_help_prints_usage() {
    let out = std::process::Command::new(REPL)
        .arg("--help")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "--help must exit 0, not panic (live: thread 'main' panicked at \
         hs-repl.rs:35, --config required): {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--config") && stdout.contains("--goal"),
        "usage names the flags a stranger needs: {stdout}"
    );
}

#[test]
fn t3_scripted_model_without_env_stays_alive_and_says_why() {
    use std::io::{BufRead as _, BufReader, Write as _};
    let mut child = std::process::Command::new(SCRIPTED)
        .env_remove("HS_SEQMODEL_SCRIPT")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdin
        .write_all(br#"{"id":1,"method":"describe","params":{}}
"#)
        .unwrap();
    stdout.read_line(&mut line).unwrap();
    assert!(
        line.contains("\"result\""),
        "describe answers without a script env (live burn: the shipped \
         example config's scripted model panicked at kernel startup, \
         killing EVERY stranger run - key or no key - with `plugin exited \
         (EOF)`): {line}"
    );
    line.clear();
    stdin
        .write_all(br#"{"id":2,"method":"model.call","params":{"prompt":"hi"}}
"#)
        .unwrap();
    stdout.read_line(&mut line).unwrap();
    assert!(
        line.contains("HS_SEQMODEL_SCRIPT"),
        "an unscripted call says what to set: {line}"
    );
    line.clear();
    stdin
        .write_all(br#"{"id":3,"method":"describe","params":{}}
"#)
        .unwrap();
    stdout.read_line(&mut line).unwrap();
    assert!(
        line.contains("\"result\""),
        "the plugin is STILL ALIVE after an unscripted call: {line}"
    );
    let _ = child.kill();
    let _ = child.wait();
}

/// T4: the OFFLINE DEMO's verifier round. The mission verifier is not a
/// plugin - it is a model call carrying the `verdict.submit` schema. A
/// dumb-replay script answers it with a replayed mission line and the
/// mission closes `verifier_malfunction` (observed live on the stranger
/// demo, 2026-09-09: `passed=true` but a broken-sounding outcome on the
/// first run a new user ever sees). The scripted model's prompt-aware
/// mode (`HS_SCRIPTED_PROMPT_AWARE=1`) answers the verdict schema honestly
/// - the offline trial docs must opt in, and this pins the outcome the
///   docs promise: `verified`.
#[test]
fn t4_offline_demo_closes_verified_in_prompt_aware_mode() {
    let _g = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let goal = "write hello.txt containing hello";
    let answer = log
        .path()
        .join("work")
        .join(goal_slug(goal))
        .join("answer.txt");
    let line1 = "{\"tool\":\"term.exec\",\"args\":{\"command\":\"printf 'hello\\n' > hello.txt && mkdir -p .hs && printf 'grep -q hello hello.txt\\n' > .hs/checks\"}}";
    let line2 = format!(
        "{{\"tool\":\"answer.submit\",\"args\":{{\"path\":\"{}\",\"summary\":\"wrote hello.txt containing hello; verified by the declared grep check\"}}}}",
        answer.display()
    );
    let script = write(dir.path(), "script.jsonl", &format!("{line1}\n{line2}\n"));
    unsafe {
        std::env::remove_var("HS_MCP_SERVERS");
        std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
        std::env::set_var("HS_SCRIPTED_PROMPT_AWARE", "1");
    }
    let mut session = ReplSession::load(&live_config(dir.path()), log.path(), false, 8)
        .expect("session load over the shipped terminal surface");
    unsafe {
        std::env::remove_var("HS_SEQMODEL_SCRIPT");
        std::env::remove_var("HS_SCRIPTED_PROMPT_AWARE");
    }
    let r = session.run_goal(goal).expect("mission runs to a result");
    assert!(r.passed, "the offline demo passes: {r:?}");
    assert_eq!(
        r.outcome, "verified",
        "the offline trial docs promise a clean close - the scripted \
         model's prompt-aware mode answers verdict.submit honestly \
         (live stranger demo without it closed verifier_malfunction): {r:?}"
    );
}

/// T5 (live burn, stranger proof 2026-09-10): a RELATIVE `--dir` - the
/// exact shape of the README quickstart and the user's pasted run
/// (`--dir tmp/hs-demo91026`) - leaks the relative anchor downstream: the
/// mission's answer artifact world_path stays relative, the world rejects
/// every proposal ("world_path must be absolute", observed 48x), and the
/// mission burns to steps_exhausted. Startup must canonicalize `--dir`
/// before any layer sees it.
#[test]
fn t5_relative_dir_is_canonicalized_at_startup() {
    let _g = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let goal = "write hello.txt containing hello";
    let answer_abs = dir
        .path()
        .join("relrun")
        .join("work")
        .join(goal_slug(goal))
        .join("answer.txt");
    let line1 = "{\"tool\":\"term.exec\",\"args\":{\"command\":\"printf 'hello\\n' > hello.txt && mkdir -p .hs && printf 'grep -q hello hello.txt\\n' > .hs/checks\"}}";
    let line2 = format!(
        "{{\"tool\":\"answer.submit\",\"args\":{{\"path\":\"{}\",\"summary\":\"wrote hello.txt containing hello; verified by the declared grep check\"}}}}",
        answer_abs.display()
    );
    let script = write(dir.path(), "script.jsonl", &format!("{line1}\n{line2}\n"));
    let out = std::process::Command::new(REPL)
        .args([
            "run",
            "--goal",
            goal,
            "--config",
            live_config(dir.path()).to_str().unwrap(),
            "--dir",
            "relrun",
        ])
        .current_dir(dir.path())
        .env("HS_SEQMODEL_SCRIPT", &script)
        .env("HS_SCRIPTED_PROMPT_AWARE", "1")
        .env("HS_TUI", "off")
        .env_remove("HS_PROJECT_ROOT")
        .env_remove("HS_PROJECT_ROOT_EFFECTIVE")
        .env_remove("HS_MCP_SERVERS")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let done = stdout
        .lines()
        .rev()
        .find(|l| l.contains("\"passed\""))
        .unwrap_or("");
    assert!(
        done.contains("\"passed\":true"),
        "relative --dir must run the stranger mission GREEN (live burn \
         2026-09-10: 48x `world rejected ... world_path must be absolute`, \
         steps_exhausted 50/50): done={done} stderr={stderr}"
    );
    assert!(
        done.contains(&format!("\"answer_path\":\"{}\"", answer_abs.display())),
        "the reported answer path is absolute after startup \
         canonicalization (live: relrun/work/... leaked into the result): \
         {done}"
    );
}

const DEEPSEEK: &str = env!("CARGO_BIN_EXE_hs-plugin-deepseek");

/// T6 (the user's fresh-install burn, 2026-09-10): with a live default
/// model and NO credential in the environment, the mission must NOT
/// start. Startup preflight fails fast: non-zero exit, stderr naming the
/// exact fix, and no mission result JSON (today: the mission burns a
/// step, ends `harness_error` with model_calls:0, and exits 0 - a
/// stranger's script cannot even detect the failure).
#[test]
fn t6_missing_credential_fails_at_startup_not_mid_mission() {
    let _g = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let cfg = write(
        dir.path(),
        "hairspring.toml",
        &format!(
            r#"
[[tools]]
name = "term.exec"
command = ["{}"]
subjects = ["*"]

[[models]]
name = "deepseek"
command = ["{DEEPSEEK}"]
default = true
subjects = ["*"]
"#,
            TERMEXEC
        ),
    );
    let out = std::process::Command::new(REPL)
        .args([
            "run",
            "--goal",
            "write hello.txt containing hello",
            "--config",
            cfg.to_str().unwrap(),
            "--dir",
            "run",
        ])
        .current_dir(dir.path())
        .env_remove("HS_DEEPSEEK_API_KEY")
        .env_remove("HS_DEEPSEEK_API_KEY_FILE")
        .env_remove("HS_DEEPSEEK_KEY_FILE")
        .env("HS_TUI", "off")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "a mission that cannot authenticate must exit non-zero (live: \
         exit 0 with harness_error JSON): {stdout}"
    );
    assert!(
        stderr.contains("HS_DEEPSEEK_API_KEY"),
        "the startup error names the exact fix (live: the actionable \
         string only appeared inside a mission-failure JSON after a \
         burned step): {stderr}"
    );
    assert!(
        !stdout.contains("\"passed\""),
        "no mission result is emitted - the mission never starts: {stdout}"
    );
    assert!(
        !stderr.contains("\\\"") && !stderr.contains("plugin  app error"),
        "the startup error is clean operator prose, not nested JSON escapes          (the user's pasted failure carried them): {stderr}"
    );
}

/// T7: run mode's exit code is the mission contract (Codex/Claude
/// convention): 0 iff the mission passed. A mission that closes
/// steps_exhausted / failed must exit non-zero (live 2026-09-10: every
/// outcome exited 0, so `hairspring run ... && echo ok` lies).
#[test]
fn t7_failed_mission_exits_nonzero() {
    let _g = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let goal = "write hello.txt containing hello";
    let answer_abs = dir
        .path()
        .join("run")
        .join("work")
        .join(goal_slug(goal))
        .join("answer.txt");
    let line1 = "{\"tool\":\"term.exec\",\"args\":{\"command\":\"printf 'hello\\n' > hello.txt && mkdir -p .hs && printf 'exit 1\\n' > .hs/checks\"}}";
    let line2 = format!(
        "{{\"tool\":\"answer.submit\",\"args\":{{\"path\":\"{}\",\"summary\":\"checks deliberately fail\"}}}}",
        answer_abs.display()
    );
    let script = write(dir.path(), "script.jsonl", &format!("{line1}\n{line2}\n"));
    let out = std::process::Command::new(REPL)
        .args([
            "run",
            "--goal",
            goal,
            "--config",
            live_config(dir.path()).to_str().unwrap(),
            "--dir",
            "run",
        ])
        .current_dir(dir.path())
        .env("HS_SEQMODEL_SCRIPT", &script)
        .env("HS_SCRIPTED_PROMPT_AWARE", "1")
        .env("HS_TUI", "off")
        .env_remove("HS_PROJECT_ROOT")
        .env_remove("HS_PROJECT_ROOT_EFFECTIVE")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let done = stdout
        .lines()
        .rev()
        .find(|l| l.contains("\"passed\""))
        .unwrap_or("");
    assert!(
        done.contains("\"passed\":false"),
        "the checks fail by construction: {done}"
    );
    assert!(
        !out.status.success(),
        "a failed mission exits non-zero (live: exit 0): {done}"
    );
}
