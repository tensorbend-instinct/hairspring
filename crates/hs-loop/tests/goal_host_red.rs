//! RED (2026-09-06, forensic item 1): the goal evaluator must run REAL
//! executable f2p commands at the checker's exec location (host) and return
//! real Pass/Fail verdicts. Pre-fix it ran them inside the repo.exec bwrap
//! sandbox, which never binds /home - mission venvs (f2p.sh execs
//! /home/sandbox/swbench/venvs/*/bin/python) died at exit 127 and every
//! goal evaluation in every session recorded env_limited.
use hs_loop::goal::{verify_verdict, GoalSpec, GoalVerdict};
use std::path::PathBuf;
use std::process::Command;

fn fixture() -> (PathBuf, PathBuf) {
    let uniq = format!(
        "goalfix-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let ws = std::env::temp_dir().join(&uniq);
    std::fs::create_dir_all(&ws).unwrap();
    let git = |args: &[&str]| {
        let o = Command::new("git").args(args).current_dir(&ws).output().unwrap();
        assert!(o.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&o.stderr));
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@t"]);
    git(&["config", "user.name", "t"]);
    std::fs::write(ws.join("code.py"), "print('base')\n").unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-qm", "base"]);
    // the fix artifact: evidence.txt, diffed by GIT (never hand-written)
    std::fs::write(ws.join("evidence.txt"), "proof\n").unwrap();
    git(&["add", "-N", "evidence.txt"]);
    let diff = Command::new("git").args(["diff"]).current_dir(&ws).output().unwrap();
    let answer = std::env::temp_dir().join(format!("{uniq}.answer"));
    std::fs::write(&answer, String::from_utf8_lossy(&diff.stdout).to_string()).unwrap();
    git(&["reset", "-q"]);
    let _ = std::fs::remove_file(ws.join("evidence.txt"));
    (ws, answer)
}

#[test]
fn goal_passes_when_acceptance_command_exits_zero() {
    let (ws, answer) = fixture();
    let g = GoalSpec { ws: ws.clone(), f2p: vec!["test -f evidence.txt".into()], timeout_secs: 60 };
    assert_eq!(verify_verdict(&g, &answer), GoalVerdict::Pass);
}

#[test]
fn goal_fails_red_not_env_limited_when_acceptance_command_fails() {
    let (ws, answer) = fixture();
    let g = GoalSpec { ws: ws.clone(), f2p: vec!["test -f absent.txt".into()], timeout_secs: 60 };
    assert_eq!(verify_verdict(&g, &answer), GoalVerdict::Fail);
}

#[test]
fn goal_runs_command_whose_interpreter_lives_under_home() {
    // mission venv shape: f2p.sh execs a python under /home/sandbox/... -
    // the bwrap sandbox never binds /home (policy), so pre-fix this is
    // exit 127 env_limited EVERY time. On the host it runs.
    let dir = PathBuf::from("/home/sandbox/.hs-goalfixture/bin");
    std::fs::create_dir_all(&dir).unwrap();
    let probe = dir.join("probe");
    std::fs::write(&probe, "#!/bin/sh\nexit 0\n").unwrap();
    let _ = Command::new("chmod").args(["+x"]).arg(&probe).status();
    let (ws, answer) = fixture();
    let g = GoalSpec { ws: ws.clone(), f2p: vec![probe.to_string_lossy().to_string()], timeout_secs: 60 };
    let v = verify_verdict(&g, &answer);
    let _ = std::fs::remove_dir_all("/home/sandbox/.hs-goalfixture");
    assert_eq!(v, GoalVerdict::Pass, "venv-under-home command must run at the goal evaluator's exec location");
}
