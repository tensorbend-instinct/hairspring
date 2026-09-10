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
        .env("HOME", dir.path())
        .env_remove("XDG_CONFIG_HOME")
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

/// T8: `hairspring setup --check` is the readiness surface a stranger (or
/// their script) consults first: per-provider status, exit 0 when any
/// live provider can run, 1 otherwise (Codex `auth status` parity).
/// Today the subcommand does not exist - it falls into flag parsing and
/// panics on `--config required`.
#[test]
fn t8_setup_check_reports_readiness() {
    let _g = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().unwrap();
    let out = std::process::Command::new(REPL)
        .args(["setup", "--check"])
        .env("HOME", home.path())
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("HS_DEEPSEEK_API_KEY")
        .env_remove("HS_DEEPSEEK_API_KEY_FILE")
        .env_remove("HS_GLM_API_KEY")
        .env_remove("HS_GLM_API_KEY_FILE")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !out.status.success(),
        "no credentials anywhere => exit 1: {stdout}"
    );
    assert!(
        stdout.contains("deepseek"),
        "readiness names the providers a stranger can set up: {stdout}"
    );
    assert!(
        stdout.contains("hairspring setup"),
        "the fix is the guided setup itself: {stdout}"
    );
}

/// T9: the guided save path (non-interactive form for scripts): the key
/// lands in the config dir with owner-only permissions, readiness flips,
/// and the same file is what the mission path's load_key finds - no env
/// export needed in a fresh shell.
#[test]
fn t9_setup_key_stdin_persists_owner_only_and_flips_readiness() {
    let _g = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().unwrap();
    let key = "sk-test-t9-never-leaves-the-box";
    let mut child = std::process::Command::new(REPL)
        .args(["setup", "--provider", "deepseek", "--key-stdin"])
        .env("HOME", home.path())
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("HS_DEEPSEEK_API_KEY")
        .env_remove("HS_DEEPSEEK_API_KEY_FILE")
        .env("HS_DEEPSEEK_BASE_URL", "http://127.0.0.1:1/chat/completions")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write as _;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(format!("{key}\n").as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "save succeeds even with validation unreachable (warn, not fail): {stdout} {stderr}"
    );
    assert!(
        !stdout.contains(key) && !stderr.contains(key),
        "the key is never echoed: {stdout} {stderr}"
    );
    let key_file = home
        .path()
        .join(".config/hairspring/keys/deepseek.key");
    let saved = std::fs::read_to_string(&key_file)
        .unwrap_or_else(|e| panic!("key file saved at {}: {e}", key_file.display()));
    assert_eq!(saved.trim(), key, "the saved key is exactly what was piped");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&key_file).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "key material is owner-only (0600), got {mode:o}"
        );
    }
    let check = std::process::Command::new(REPL)
        .args(["setup", "--check"])
        .env("HOME", home.path())
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("HS_DEEPSEEK_API_KEY")
        .env_remove("HS_DEEPSEEK_API_KEY_FILE")
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "readiness flips to ready after setup - the fresh-shell contract: {}",
        String::from_utf8_lossy(&check.stdout)
    );
}

/// T5: the README's OFFLINE TRIAL, executed verbatim end to end through
/// the real `hs-repl` binary, the SHIPPED example config template, and
/// the SHIPPED demo script (regression lock for the documented stranger
/// path). Live runs on 2026-09-10 proved the trial was broken as
/// documented: the checker's phase-2 critic (hs-plugin-critic) had no
/// zero-network model - HS_CRITIC_MODEL accepts only deepseek/glm - so
/// with no key every answer.submit fail-closed, and even the scripted
/// stand-in needed HS_CRITIC_SCRIPT exported (absent from the docs).
/// The docs now carry the full export set; this test pins the promised
/// outcome: the mission closes `verified`.
#[test]
fn t5_readme_offline_trial_closes_verified() {
    let _g = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().unwrap();
    // The shipped demo script hardcodes /tmp/hs-demo (the README tells
    // the stranger the same --dir); start clean.
    let demo = std::path::Path::new("/tmp/hs-demo");
    let _ = std::fs::remove_dir_all(demo);
    std::fs::create_dir_all(demo).unwrap();

    // The config install.sh writes: the shipped template with @PREFIX@
    // resolved and the default flipped from deepseek to scripted,
    // exactly as the README instructs (comment one, uncomment other).
    let prefix = std::path::Path::new(SCRIPTED).parent().unwrap().to_path_buf();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let mut toml = std::fs::read_to_string(root.join("hairspring.example.toml"))
        .expect("shipped example template");
    let deepseek_default =
        "name = \"deepseek\"\ncommand = [\"@PREFIX@/bin/hs-plugin-deepseek\"]\ndefault = true";
    let deepseek_nodefault =
        "name = \"deepseek\"\ncommand = [\"@PREFIX@/bin/hs-plugin-deepseek\"]";
    assert!(toml.contains(deepseek_default), "template deepseek stanza");
    toml = toml.replacen(deepseek_default, deepseek_nodefault, 1);
    assert_eq!(toml.matches("# default = true").count(), 1, "one scripted default marker");
    toml = toml.replacen("# default = true", "default = true", 1);
    // The template installs plugins under @PREFIX@/bin; the debug
    // workspace keeps them at target/debug - point @PREFIX@/bin there.
    toml = toml.replace("@PREFIX@/bin", &prefix.display().to_string());
    toml = toml.replace("@PREFIX@", &prefix.display().to_string());
    let cfgdir = tempfile::tempdir().unwrap();
    let cfg = write(cfgdir.path(), "hairspring.toml", &toml);
    let script = root.join("examples/seqmodel-demo.jsonl");
    assert!(script.is_file(), "shipped demo script");

    let out = std::process::Command::new(REPL)
        .args([
            "run",
            "--goal",
            "write hello.txt containing hello",
            "--config",
            &cfg.display().to_string(),
            "--dir",
            "/tmp/hs-demo",
        ])
        .env("HOME", home.path())
        .env("HS_SEQMODEL_SCRIPT", &script)
        .env("HS_SCRIPTED_PROMPT_AWARE", "1")
        .env("HS_CRITIC_SCRIPT", "tool:grep -q hello hello.txt|clean")
        .env_remove("HS_MCP_SERVERS")
        .env_remove("HS_CRITIC_MODEL")
        .env_remove("HS_DEEPSEEK_API_KEY")
        .env_remove("HS_DEEPSEEK_API_KEY_FILE")
        .env_remove("HS_GLM_API_KEY")
        .env_remove("HS_GLM_API_KEY_FILE")
        .output()
        .expect("hs-repl run spawns");
    assert!(
        out.status.success(),
        "the documented offline trial must exit 0: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The stream segments are a binary log; read lossy (as `strings` does)
    // and scan every stream (mission + critic + verifier).
    let mut log = String::new();
    for e in std::fs::read_dir(demo.join("streams")).expect("streams dir") {
        let seg = e.unwrap().path().join("seg-000000.hslog");
        if seg.is_file() {
            let bytes = std::fs::read(&seg).unwrap();
            log.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    assert!(
        log.contains("\"passed\":true"),
        "the documented trial passes: {log}"
    );
    assert!(
        log.contains("\"outcome\":\"verified\""),
        "the documented trial closes VERIFIED (live 2026-09-10: without          HS_CRITIC_SCRIPT the critic fail-closed every submission; without          HS_SCRIPTED_PROMPT_AWARE the audit closed verifier_malfunction): {log}"
    );
}
