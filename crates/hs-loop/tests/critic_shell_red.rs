//! RED: the critic's shell claimed read-only ("RULES: ... never modify,
//! move, or delete") while `termexec::run` gave it an unrestricted ROOT
//! shell - a harness-model trust gap found in the 2026-09-09 deep pass.
//! Mechanism must match promise: the critic's command surface runs
//! UNPRIVILEGED (uid/gid nobody, supplementary groups cleared), so task
//! files are read-only BY MECHANISM and only world-writable scratch
//! (/tmp) is writable. The agent's own mission surface (`termexec::run`)
//! is unchanged - the author is supposed to mutate; the verifier is not.

/// P1 (premise): the surface the critic used pre-fix - `termexec::run` as
/// root - CAN clobber a task deliverable. This is the hole being closed;
/// kept green as proof the premise was real.
#[test]
fn p1_premise_root_surface_could_mutate() {
    let dir = tempfile::tempdir().unwrap();
    let victim = dir.path().join("deliverable.txt");
    std::fs::write(&victim, "original\n").unwrap();
    let r = hs_loop::termexec::run(dir.path(), "echo pwned > deliverable.txt", 10);
    assert_eq!(r["exit_code"], 0, "{r}");
    assert_eq!(
        std::fs::read_to_string(&victim).unwrap(),
        "pwned\n",
        "premise: the old root shell mutates task files"
    );
}

fn traversable(dir: &tempfile::TempDir) -> std::path::PathBuf {
    // Deployment reality: task workdirs (/app, log work dirs) are
    // world-traversable (0755); tempfile::tempdir is 0700, which would
    // deny the unprivileged critic traversal for the wrong reason.
    let mut perms = std::fs::metadata(dir.path()).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
    }
    std::fs::set_permissions(dir.path(), perms).unwrap();
    dir.path().to_path_buf()
}

/// R1: the read-only surface REFUSES mutation of a task deliverable.
#[test]
fn r1_readonly_refuses_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let ws = traversable(&dir);
    let victim = ws.join("deliverable.txt");
    std::fs::write(&victim, "original\n").unwrap();
    let r = hs_loop::termexec::run_readonly(&ws, "echo pwned > deliverable.txt", 10);
    assert_ne!(r["exit_code"].as_i64().unwrap_or(0), 0, "write must fail: {r}");
    assert_eq!(
        std::fs::read_to_string(&victim).unwrap(),
        "original\n",
        "deliverable untouched: {r}"
    );
    let err = r["stderr"].as_str().unwrap_or("").to_lowercase();
    assert!(err.contains("permission denied"), "says why: {r}");
}

/// R2: the read-only surface still READS (the critic's real job).
#[test]
fn r2_readonly_reads_task_files() {
    let dir = tempfile::tempdir().unwrap();
    let ws = traversable(&dir);
    std::fs::write(ws.join("results.txt"), "beta-ok\n").unwrap();
    let r = hs_loop::termexec::run_readonly(&ws, "cat results.txt", 10);
    assert_eq!(r["exit_code"], 0, "{r}");
    assert!(r["stdout"].as_str().unwrap().contains("beta-ok"), "{r}");
}

/// R3: /tmp scratch stays writable (the contract's scratch allowance).
#[test]
fn r3_readonly_tmp_scratch_works() {
    let dir = tempfile::tempdir().unwrap();
    let ws = traversable(&dir);
    let probe = format!("/tmp/hs-critic-scratch-{}", std::process::id());
    let r = hs_loop::termexec::run_readonly(
        &ws,
        &format!("echo ok > {probe} && cat {probe} && rm -f {probe}"),
        10,
    );
    assert_eq!(r["exit_code"], 0, "{r}");
    assert!(r["stdout"].as_str().unwrap().contains("ok"), "{r}");
}

/// R4: the contract text matches the mechanism - no "you are root",
/// no unenforced read-only claim.
#[test]
fn r4_prompt_matches_mechanism() {
    let sys = hs_loop::critic::CRITIC_SYSTEM;
    assert!(
        !sys.contains("you are root"),
        "the prompt must not claim privileges the mechanism removed"
    );
    assert!(
        sys.contains("unprivileged"),
        "the prompt states the enforced posture"
    );
    assert!(sys.contains("read-only"), "the prompt states read-only");
}

/// R5: the refute LOOP routes through the read-only surface - a
/// (hostile or careless) critic model issuing a mutation cannot damage
/// the submission under review.
#[test]
fn r5_critic_loop_cannot_mutate_submission() {
    let dir = tempfile::tempdir().unwrap();
    let ws = traversable(&dir);
    let victim = ws.join("results.txt");
    std::fs::write(&victim, "score 0.97\n").unwrap();
    let mut m = hs_loop::critic::ScriptedCritic::new(vec![
        hs_loop::critic::CriticReply::ToolCalls(vec![(
            "c1".into(),
            "echo pwned > results.txt; rm -f results.txt".into(),
        )]),
        hs_loop::critic::CriticReply::Final(
            "{\"refuted\": false, \"reason\": \"probed and re-derived\"}".into(),
        ),
    ]);
    let _r = hs_loop::critic::refute(
        &ws, "Write score 0.97 to results.txt.", "cat results.txt",
        &hs_loop::critic::RefuteConfig::default(), &mut m,
    );
    assert_eq!(
        std::fs::read_to_string(&victim).unwrap(),
        "score 0.97\n",
        "critic commands cannot mutate the submission under review"
    );
}

/// R6: fail-closed when privilege cannot be dropped (non-root caller):
/// the surface refuses rather than running unenforced.
#[test]
fn r6_readonly_fails_closed_without_root() {
    let dir = tempfile::tempdir().unwrap();
    let ws = traversable(&dir);
    let euid = std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find(|l| l.starts_with("Uid:"))
        .and_then(|l| l.split_whitespace().nth(2))
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);
    let r = hs_loop::termexec::run_readonly(&ws, "echo should-not-run-as-unenforced", 10);
    if euid == 0 {
        assert_eq!(r["exit_code"], 0, "root can enforce: {r}");
    } else {
        assert!(
            r.get("$error").is_some() || r["exit_code"].as_i64().unwrap_or(0) != 0,
            "no unenforced run when uid-drop is impossible: {r}"
        );
    }
}
