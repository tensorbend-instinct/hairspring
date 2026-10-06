//! RED: first-run readiness gate + explicit project root (Eric
//! 2026-09-10 stranger bar, live reports: no wizard on a fresh clone,
//! mission work silently landing in the machine's /tmp).
//!
//! - the configured DEFAULT model must be offline or credentialed BEFORE
//!   any mission machinery starts; non-TTY gets an actionable error
//!   naming `hairspring setup`, never a mid-mission provider 400;
//! - the project root is resolved once at startup and printed in EVERY
//!   mode; a TTY run without --project-dir gets one prompt.

use std::path::{Path, PathBuf};
use std::process::Command;

const HS_REPL: &str = env!("CARGO_BIN_EXE_hs-repl");

fn write_rig(dir: &Path, default_first: bool) -> PathBuf {
    // Minimal rig: two [[models]] stanzas, no tools - the gate reads the
    // default model name before any session machinery, and the gate
    // error must arrive before plugin spawn.
    let (a, b) = if default_first {
        (
            "[[models]]\nname = \"deepseek\"\ncommand = [\"/nonexistent/hs-plugin-deepseek\"]\ndefault = true\nsubjects = [\"*\"]\n",
            "[[models]]\nname = \"scripted\"\ncommand = [\"/nonexistent/hs-plugin-scripted\"]\nsubjects = [\"*\"]\n",
        )
    } else {
        (
            "[[models]]\nname = \"scripted\"\ncommand = [\"/nonexistent/hs-plugin-scripted\"]\ndefault = true\nsubjects = [\"*\"]\n",
            "[[models]]\nname = \"deepseek\"\ncommand = [\"/nonexistent/hs-plugin-deepseek\"]\nsubjects = [\"*\"]\n",
        )
    };
    let p = dir.join("rig.toml");
    std::fs::write(&p, format!("{a}\n{b}")).unwrap();
    p
}

fn clean_env(cmd: &mut Command, home: &Path) {
    cmd.env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("HS_DEEPSEEK_API_KEY")
        .env_remove("HS_DEEPSEEK_API_KEY_FILE")
        .env_remove("HS_GLM_API_KEY")
        .env_remove("HS_GLM_API_KEY_FILE")
        .env_remove("HS_PROJECT_ROOT")
        .env_remove("HS_PROJECT_ROOT_EFFECTIVE");
}

#[test]
fn g1_nontty_deepseek_default_without_key_refuses_before_any_mission() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let rig = write_rig(tmp.path(), true);
    let run = tmp.path().join("run");
    let mut cmd = Command::new(HS_REPL);
    cmd.args([
        "run",
        "--goal",
        "x",
        "--config",
        rig.to_str().unwrap(),
        "--dir",
        run.to_str().unwrap(),
    ]);
    clean_env(&mut cmd, &home);
    let out = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "no credential must refuse: {stderr}");
    assert!(
        stderr.contains("hairspring setup"),
        "the refusal names the guided remedy, not a provider 400: {stderr}"
    );
    assert!(
        stderr.contains("HS_DEEPSEEK_API_KEY"),
        "the refusal names the env path: {stderr}"
    );
    assert!(
        stdout.contains("project root:"),
        "the confinement root prints in EVERY mode (this run defaulted): {stdout}"
    );
    assert!(
        !run.join("streams").exists(),
        "the gate fires before any mission stream opens"
    );
}

#[test]
fn g2_scripted_default_passes_the_gate() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let rig = write_rig(tmp.path(), false);
    let run = tmp.path().join("run");
    let mut cmd = Command::new(HS_REPL);
    cmd.args([
        "run",
        "--goal",
        "x",
        "--config",
        rig.to_str().unwrap(),
        "--dir",
        run.to_str().unwrap(),
    ]);
    clean_env(&mut cmd, &home);
    let out = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("hairspring setup"),
        "the offline scripted default needs no credential: {stderr}"
    );
}

#[test]
fn g3_env_key_passes_the_gate() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let rig = write_rig(tmp.path(), true);
    let run = tmp.path().join("run");
    let mut cmd = Command::new(HS_REPL);
    cmd.args([
        "run",
        "--goal",
        "x",
        "--config",
        rig.to_str().unwrap(),
        "--dir",
        run.to_str().unwrap(),
    ])
    ;
    clean_env(&mut cmd, &home);
    cmd.env("HS_DEEPSEEK_API_KEY", "dummy-for-gate-test");
    let out = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("no credential"),
        "a resolvable env key passes the gate (fails later on the dummy): {stderr}"
    );
}

#[test]
fn g4_resolve_project_root_matrix() {
    let tmp = tempfile::tempdir().unwrap();
    let run = tmp.path().join("run");
    std::fs::create_dir_all(&run).unwrap();
    let run = run.canonicalize().unwrap();

    // default, non-interactive: <run>/work, created
    let root = hs_loop::projectroot::resolve_project_root(None, &run, None).unwrap();
    assert_eq!(root, run.join("work").canonicalize().unwrap());
    assert!(root.is_dir());

    // explicit existing dir wins, untouched by the prompt
    let proj = tmp.path().join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    let mut ask = |_d: &str| -> Result<String, String> { panic!("prompt must not fire for explicit") };
    let root = hs_loop::projectroot::resolve_project_root(Some(&proj), &run, Some(&mut ask)).unwrap();
    assert_eq!(root, proj.canonicalize().unwrap());

    // explicit missing path is an error, not a silent create
    let err = hs_loop::projectroot::resolve_project_root(
        Some(Path::new("/nonexistent/nowhere")),
        &run,
        None,
    )
    .unwrap_err();
    assert!(err.contains("--project-dir"), "{err}");

    // interactive empty answer takes the default
    let mut empty = |_d: &str| -> Result<String, String> { Ok("\n".into()) };
    let root = hs_loop::projectroot::resolve_project_root(None, &run, Some(&mut empty)).unwrap();
    assert_eq!(root, run.join("work").canonicalize().unwrap());

    // interactive typed path resolves canonical
    let typed = proj.to_string_lossy().to_string();
    let mut give = move |_d: &str| -> Result<String, String> { Ok(typed.clone()) };
    let root = hs_loop::projectroot::resolve_project_root(None, &run, Some(&mut give)).unwrap();
    assert_eq!(root, proj.canonicalize().unwrap());
}
