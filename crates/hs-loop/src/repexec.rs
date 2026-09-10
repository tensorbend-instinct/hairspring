//! repo.exec backend (Eric's ruling via iMessage 2026-09-10): the model may
//! run ANY command - open shell, zero allowlist, NO command ACL. The hard
//! boundary is the FILESYSTEM: no writes or deletes outside the project
//! workspace. The patch is applied to a scratch git worktree (live ws never
//! mutated; checker semantics unchanged) and the command runs inside bwrap:
//! system roots read-only, /ws and /tmp the writable floor, private
//! mount/pid/ipc namespaces, host home and mission env never enter the
//! sandbox, rlimits + hard timeout cap resources.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const OUT_TAIL: usize = 8192;

fn tail(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    crate::msgfmt::tail_bytes_safe(&s, OUT_TAIL)
}

/// The sandbox command line, as an argv vector (pure, unit-testable).
/// Workspace floor (Eric 2026-09-10): the mission sees the real box but can
/// only WRITE to its workspace - system roots are read-only binds, scratch
/// is rw at /ws, /tmp is a private tmpfs. Reads stay wide (compilers, apt
/// caches, headers all resolve); writes/deletes outside /ws and /tmp fail
/// with EROFS. What stays OUT entirely is host secret material: /home and
/// /mnt are never bound, /root/.ssh is tmpfs'd over, /root/.git-credentials
/// is masked with an empty file, and --clearenv keeps mission env (HS_*,
/// keys) out. pid/ipc stay unshared so a mission cannot signal or shm-snoop
/// harness processes.
#[must_use]
pub fn sandbox_argv(scratch: &Path, _out_f: &Path, _err_f: &Path, cmd: &str) -> Vec<String> {
    // out/err paths inside the sandbox: the scratch is mounted at /ws
    // Group the command: `a; b >file` redirects ONLY the last simple
    // command, which used to silently lose every earlier command's output
    // (they went to the null'd child stdout). Compound commands are the
    // common case for real shells.
    let script = format!(
        "{{ {cmd}
}} >/ws/.repexec-out 2>/ws/.repexec-err"
    );
    let mut v: Vec<String> = [
        "prlimit",
        "--as=8589934592",
        "--nproc=512",
        "--fsize=8589934592",
        "--nofile=4096",
        "--",
        "bwrap",
        "--unshare-pid",
        "--unshare-ipc",
        "--die-with-parent",
        "--clearenv",
        "--ro-bind",
        "/usr",
        "/usr",
        "--ro-bind",
        "/bin",
        "/bin",
    ]
    .iter()
    .map(std::string::ToString::to_string)
    .collect();
    // Egress switch (2026-09-06, contamination re-baseline prep): default is
    // host network; HS_SWE_NET=off drops --share-net so bwrap unshares the
    // net namespace - no outbound, no loopback. Gold-patch fetching dies.
    if !egress_off() {
        v.push("--share-net".into());
    }
    for d in ["/lib", "/lib64", "/etc", "/var", "/opt", "/root"] {
        if Path::new(d).exists() {
            v.push("--ro-bind".into());
            v.push(d.into());
            v.push(d.into());
        }
    }
    // Blessed toolchain caches (Eric 2026-09-10: "package caches are fine
    // where the tools need them"): uv/pip ($HOME/.cache), cargo
    // registry+git ($HOME/.cargo), npm ($HOME/.npm), user-level installs
    // ($HOME/.local), go modules ($HOME/go). Later binds override the ro
    // /root beneath them; created on the host when missing so missions can
    // always build envs. The rest of $HOME stays read-only.
    for d in [
        "/root/.cache",
        "/root/.cargo",
        "/root/.npm",
        "/root/.local",
        "/root/go",
    ] {
        if std::fs::create_dir_all(d).is_ok() {
            v.push("--bind".into());
            v.push(d.into());
            v.push(d.into());
        }
    }
    // mask host secret material that sits under the bound roots (a
    // zero-length regular file: /dev/null as a bind source is EACCES under
    // this box's device policy)
    let mask = scratch.join(".repexec-mask");
    let _ = std::fs::write(&mask, b"");
    // DNS: /etc/resolv.conf is commonly a symlink into /run
    // (systemd-resolved) and /run is never bound, so the symlink dangles
    // and every lookup dies ("Temporary failure in name resolution").
    // Copy the RESOLVED contents into the sandbox at the canonical path.
    if let Ok(target) = std::fs::canonicalize("/etc/resolv.conf") {
        if let Ok(bytes) = std::fs::read(&target) {
            let resolv = scratch.join(".repexec-resolv.conf");
            if std::fs::write(&resolv, bytes).is_ok() {
                v.extend([
                    "--ro-bind".into(),
                    resolv.display().to_string(),
                    target.display().to_string(),
                ]);
            }
        }
    }
    if Path::new("/root/.git-credentials").exists() {
        v.extend([
            "--ro-bind".into(),
            mask.display().to_string(),
            "/root/.git-credentials".into(),
        ]);
    }
    if Path::new("/root/.ssh").exists() {
        v.extend(["--tmpfs".into(), "/root/.ssh".into()]);
    }
    v.extend([
        "--dev-bind".into(),
        "/dev".into(),
        "/dev".into(),
        "--proc".into(),
        "/proc".into(),
        "--bind".into(),
        scratch.display().to_string(),
        "/ws".into(),
        "--tmpfs".into(),
        "/tmp".into(),
        "--chdir".into(),
        "/ws".into(),
        "--setenv".into(),
        "PATH".into(),
        "/root/.cargo/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin".into(),
        "--setenv".into(),
        "HOME".into(),
        "/root".into(),
        "--setenv".into(),
        "LANG".into(),
        "C.UTF-8".into(),
        "--".into(),
        "sh".into(),
        "-c".into(),
        script,
    ]);
    v
}

/// Extract one unified diff from model-supplied text: a `diff` fence, or
/// the raw diff itself (starts with "diff --git" or "--- ").
#[must_use]
pub fn extract_diff(raw: &str) -> Option<String> {
    if let Some(p) = hs_bench::extract_patch(raw) {
        return Some(p);
    }
    // trim_end_matches('\n'), never trim(): a hunk's final context line can
    // be a single space (blank source line) and trimming it corrupts the
    // hunk ("error: corrupt patch at line N", octodns-1298 2026-09-07).
    let t = raw.trim_start().trim_end_matches('\n');
    // git apply rejects a patch whose last hunk line lacks the trailing
    // newline ("corrupt patch at line N") - always re-terminate.
    (t.starts_with("diff --git") || t.starts_with("--- ")).then(|| format!("{t}\n"))
}

/// Shared prep: read the answer, extract the diff, make a scratch worktree,
/// apply the patch there. Ok(None) = clean feedback result (no patch / no
/// diff / does not apply); Err = machinery failure result.
fn prep(ws: &Path, answer_path: &Path) -> Result<Option<PathBuf>, Value> {
    let raw = match std::fs::read_to_string(answer_path) {
        Ok(s) => s,
        Err(_) => {
            return Err(
                json!({"applied": false, "note": "repo.exec runs build/test against the candidate patch written by edit.patch. No patch exists yet - write your fix with edit.patch first, then run repo.exec again (or pass args.diff inline)"}),
            );
        }
    };
    let Some(patch) = extract_diff(&raw) else {
        return Err(
            json!({"applied": false, "note": "no diff found in the current answer - wrap one unified diff in a `diff` fence"}),
        );
    };
    prep_diff(ws, &patch)
}

/// Test-only fault injection for the transient-spawn class (flake hunt,
/// suite-m46): the first N `prep_attempt` calls to git fail with `WouldBlock`,
/// letting a RED test pin the retry recovery deterministically.
fn transient_spawn_failures_left() -> usize {
    std::env::var("HS_REPEXEC_TEST_FAIL_GIT_SPAWNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn git_spawn(ws: &Path, args: &[&str]) -> Result<std::process::Output, std::io::Error> {
    static FAILS_LEFT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);
    let left = FAILS_LEFT.load(std::sync::atomic::Ordering::Relaxed);
    let cfg = transient_spawn_failures_left();
    // usize::MAX = uninitialized; latch the env value exactly once.
    if left == usize::MAX {
        FAILS_LEFT.store(cfg, std::sync::atomic::Ordering::Relaxed);
    }
    if FAILS_LEFT
        .fetch_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |v| (v > 0).then(|| v - 1),
        )
        .is_ok()
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "injected transient spawn failure (HS_REPEXEC_TEST_FAIL_GIT_SPAWNS)",
        ));
    }
    Command::new("git").args(args).current_dir(ws).output()
}

/// One worktree-add attempt; the error Value is the machinery arm.
fn worktree_add_once(ws: &Path, scratch: &Path) -> Result<(), Value> {
    match git_spawn(ws, &["worktree", "add", "--detach", &scratch.display().to_string(), "HEAD"])
    {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(json!({"$error": format!("scratch worktree: {}", String::from_utf8_lossy(&o.stderr))})),
        Err(e) => Err(json!({"$error": format!("scratch worktree: {e}")})),
    }
}

/// Prep from a diff the model supplies inline (T4: test before the first
/// answer.write). Same scratch-worktree semantics as prep.
/// One prep-gradient scratch name. The wall clock is NOT a uniqueness
/// source: on this fleet, `clock_gettime` returns IDENTICAL nanos to two
/// threads (~30k duplicates per 1.6M samples measured), so pid+nanos
/// scratch paths collided - two threads raced git's check-then-create
/// `worktree add`, one thread's `git apply` landed in the other dir,
/// and the suite flaked with a "patch does not apply" verdict arm of a
/// DIFFERENT call (m51). A process-local counter is the only honest
/// uniqueness primitive; the pid component keeps names distinct across
/// test binaries, and the pre-add `worktree remove` still clears any
/// same-tagged leftover from a pid-recycled earlier run.
pub fn unique_tag(prefix: &str) -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{prefix}-{}-{n}", std::process::id())
}

fn prep_diff(ws: &Path, patch: &str) -> Result<Option<PathBuf>, Value> {
    let tag = unique_tag("repexec");
    let scratch = std::env::temp_dir().join(&tag);
    let _ = Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(&scratch)
        .current_dir(ws)
        .output();
    // Transient machinery failures (spawn EAGAIN, one-shot worktree errors)
    // get bounded retries instead of failing the mission on a load hiccup;
    // $error machinery arms ONLY - verdict arms (applied:false) never retry.
    let mut last_err: Option<Value> = None;
    let mut added = false;
    for attempt in 0..3u32 {
        match worktree_add_once(ws, &scratch) {
            Ok(()) => {
                added = true;
                break;
            }
            Err(e) => {
                last_err = Some(e);
                if attempt < 2 {
                    std::thread::sleep(std::time::Duration::from_millis(25 * (u64::from(attempt) + 1)));
                }
            }
        }
    }
    if !added {
        return Err(last_err.unwrap_or_else(|| json!({"$error": "scratch worktree: retries exhausted"})));
    }
    let mut last_apply_err: Option<Value> = None;
    for attempt in 0..3u32 {
        match hs_bench::apply_model_patch(&scratch, patch) {
            Ok(hs_bench::ApplyResult::Applied) => return Ok(Some(scratch)),
            Ok(hs_bench::ApplyResult::NoApply(msg)) => {
                cleanup(ws, &scratch);
                return Err(json!({"applied": false, "apply_error": msg}));
            }
            Err(e) => {
                last_apply_err = Some(json!({"$error": format!("apply machinery: {e:?}")}));
                if attempt < 2 {
                    std::thread::sleep(std::time::Duration::from_millis(25 * (u64::from(attempt) + 1)));
                }
            }
        }
    }
    cleanup(ws, &scratch);
    Err(last_apply_err.unwrap_or_else(|| json!({"$error": "apply machinery: retries exhausted"})))
}

fn cleanup(ws: &Path, scratch: &Path) {
    cleanup_scratch(ws, scratch, true);
}

fn cleanup_scratch(ws: &Path, scratch: &Path, worktree: bool) {
    if worktree {
        let _ = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(scratch)
            .current_dir(ws)
            .output();
        let _ = Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(ws)
            .output();
    } else {
        let _ = std::fs::remove_dir_all(scratch);
    }
}

/// Edit-path guardrail (post-B7, 2026-09-05): repo.exec is build/test ONLY -
/// every source edit goes through edit.apply. B7's model bypassed the splice
/// path by hand-writing raw diffs and git-applying them through this shell,
/// and the checker harvested "corrupt patch at line 172" - the
/// model-written-diff failure class through the exec backdoor. Returns
/// Some(reason) when the command invokes `git apply` (the whole class,
/// --check included) or writes a raw .diff/.patch file (redirection, tee,
/// cp/mv/install destination). Reads of diff files, `git diff` to stdout,
/// and every other command stay allowed.
#[must_use]
pub fn edit_path_violation(command: &str) -> Option<String> {
    let is_diff_target = |t: &str| {
        let t = t.trim_matches(|c| c == '"' || c == '\'');
        std::path::Path::new(t)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("diff") || e.eq_ignore_ascii_case("patch"))
    };
    // Per simple-command segment (split on shell operators) so arguments of
    // one command are never attributed to another.
    for segment in command.split(['|', ';', '&', '(', ')']) {
        let mut toks: Vec<String> = Vec::new();
        for raw in segment.split_whitespace() {
            // split attached redirections: ">f", "2>f", "2>>f", "2>f" style
            if let Some(pos) = raw.find(['>', '<']) {
                let (op, target) = raw.split_at(pos + 1);
                let ok = op
                    .chars()
                    .all(|c| c == '>' || c == '<' || c.is_ascii_digit())
                    && (op.contains('>') || op.contains('<'));
                if ok && !target.is_empty() {
                    toks.push(op.to_string());
                    toks.push(target.to_string());
                    continue;
                }
            }
            toks.push(raw.to_string());
        }
        // git apply: skip global options (-C dir, -c k=v, --git-dir=..., flags)
        for (i, t) in toks.iter().enumerate() {
            if t == "git" {
                let mut j = i + 1;
                while j < toks.len() {
                    let g = toks[j].as_str();
                    if matches!(
                        g,
                        "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" | "--exec-path"
                    ) {
                        j += 2;
                    } else if g.starts_with('-') {
                        j += 1;
                    } else {
                        break;
                    }
                }
                if toks.get(j).map(std::string::String::as_str) == Some("apply") {
                    return Some("git apply invocation".to_string());
                }
            }
        }
        // redirection writes to .diff/.patch
        for (i, t) in toks.iter().enumerate() {
            if t.contains('>') && t.chars().all(|c| c == '>' || c.is_ascii_digit())
                && let Some(target) = toks.get(i + 1)
                    && is_diff_target(target) {
                        return Some(format!("raw diff-file write ({t} {target})"));
                    }
        }
        // tee writes (all operands are write targets)
        if let Some(i) = toks.iter().position(|t| t == "tee") {
            for a in &toks[i + 1..] {
                if a.starts_with('-') {
                    continue;
                }
                if is_diff_target(a) {
                    return Some(format!("raw diff-file write (tee {a})"));
                }
            }
        }
        // cp / mv / install: destination is the last operand
        if let Some(i) = toks
            .iter()
            .position(|t| matches!(t.as_str(), "cp" | "mv" | "install"))
            && let Some(dst) = toks[i + 1..].iter().rfind(|a| !a.starts_with('-'))
                && is_diff_target(dst) {
                    return Some(format!("raw diff-file write ({} {dst})", toks[i]));
                }
    }
    None
}

/// The gate result every repo.exec entry point returns on a violation:
/// nothing executed, nothing applied, steering that names edit.apply.
fn edit_gate(command: &str, patch_mode: bool) -> Option<Value> {
    edit_path_violation(command).map(|reason| {
        let class = violation_class(&reason);
        json!({"applied": false, "scratch": !patch_mode, "timed_out": false, "exit_code": -1,
               "stdout": "", "stderr": "",
               "error": format!("forbidden edit path (class: {class}, {reason}): repo.exec is build/test only. Make ALL edits with edit.apply - example: edit.apply {{\"edits\":[{{\"path\":\"src/file.py\",\"search\":\"<exact old text>\",\"replace\":\"<new text>\"}}]}}; op=\"diff\" shows the cumulative diff. git apply and raw .diff/.patch file writes are rejected here, and repeated attempts of the same class are counted and escalate.")})
    })
}

/// Open-shell exec in the sandbox. `command` is arbitrary by design.
#[must_use]
pub fn run_sandboxed(ws: &Path, answer_path: &Path, command: &str, timeout_secs: u64) -> Value {
    if let Some(v) = edit_gate(command, true) {
        return v;
    }
    run_with_prep(prep(ws, answer_path), ws, command, timeout_secs, true)
}

/// Open-shell exec against an inline diff (T4: test-before-first-submit).
#[must_use]
pub fn run_sandboxed_with_diff(ws: &Path, diff: &str, command: &str, timeout_secs: u64) -> Value {
    if let Some(v) = edit_gate(command, true) {
        return v;
    }
    let Some(patch) = extract_diff(diff) else {
        return json!({"applied": false, "note": "no unified diff in args.diff - pass one unified diff, raw or in a `diff` fence"});
    };
    run_with_prep(prep_diff(ws, &patch), ws, command, timeout_secs, true)
}

/// Scratch-shell mode (Eric 2026-09-05, post-verify17092): the model uses
/// repo.exec as a general shell (git log, grep, pwd) with NO candidate diff.
/// Contract: run against a pristine self-contained clone of the workspace
/// (a worktree's .git pointer would dangle inside the bwrap mount ns, so
/// this is a hardlinked local clone, not a worktree - git works inside the
/// sandbox). applied=false + scratch=true: this is exploration, never
/// candidate verification - only the diff paths count toward the answer
/// gate.
pub fn run_sandboxed_no_patch(ws: &Path, command: &str, timeout_secs: u64) -> Value {
    if let Some(v) = edit_gate(command, false) {
        return v;
    }
    run_with_prep(
        scratch_clone(ws).map(Some),
        ws,
        command,
        timeout_secs,
        false,
    )
}

/// Self-contained scratch copy for scratch-shell mode: `git clone --local`
/// hardlinks objects, so even a large repo copies fast, and the result has
/// a real .git directory that survives the sandbox bind at /ws.
fn scratch_clone(ws: &Path) -> Result<PathBuf, Value> {
    let scratch = std::env::temp_dir().join(unique_tag("repsh"));
    let _ = std::fs::remove_dir_all(&scratch);
    match Command::new("git")
        .args(["clone", "--quiet", "--local", "--no-hardlinks"])
        .arg(ws)
        .arg(&scratch)
        .output()
    {
        Ok(o) if o.status.success() => Ok(scratch),
        Ok(o) => {
            Err(json!({"$error": format!("scratch clone: {}", String::from_utf8_lossy(&o.stderr))}))
        }
        Err(e) => Err(json!({"$error": format!("scratch clone: {e}")})),
    }
}

fn run_with_prep(
    prepped: Result<Option<PathBuf>, Value>,
    ws: &Path,
    command: &str,
    timeout_secs: u64,
    patch_mode: bool,
) -> Value {
    let scratch = match prepped {
        Ok(Some(s)) => s,
        Ok(None) => unreachable!(),
        Err(early) => return early,
    };
    let out_f = scratch.join(".repexec-out");
    let err_f = scratch.join(".repexec-err");
    let argv = sandbox_argv(&scratch, &out_f, &err_f, command.trim());
    let child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            cleanup(ws, &scratch);
            return json!({"$error": format!("spawn sandbox: {e}")});
        }
    };
    let t0 = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(timeout_secs);
    let (status, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(st)) => break (Some(st), false),
            Ok(None) if t0.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break (None, true);
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(e) => {
                cleanup(ws, &scratch);
                return json!({"$error": format!("wait: {e}")});
            }
        }
    };
    let stdout = tail(&std::fs::read(&out_f).unwrap_or_default());
    let stderr = tail(&std::fs::read(&err_f).unwrap_or_default());
    cleanup_scratch(ws, &scratch, patch_mode);
    if timed_out {
        return json!({"applied": patch_mode, "scratch": !patch_mode, "timed_out": true, "timeout_secs": timeout_secs,
                      "stdout": stdout, "stderr": stderr});
    }
    json!({"applied": patch_mode, "scratch": !patch_mode, "timed_out": false,
           "exit_code": status.and_then(|s| s.code()).unwrap_or(-1),
           "stdout": stdout, "stderr": stderr})
}

/// Host-side exec for harness-generated acceptance commands (the goal
/// evaluator's f2p). The bwrap sandbox exists to contain MODEL commands;
/// the goal evaluator runs harness-fixed commands, and mission venvs live
/// under /home which bwrap never binds (forensic item 1, 2026-09-06: every
/// session's goal evaluation died at exit 127 and recorded `env_limited`).
/// Same scratch-worktree prep, same timeout discipline, exec on the host
/// exactly where the standalone checker runs.
/// Egress-off predicate (contamination re-baseline, 2026-09-06).
#[must_use]
pub fn egress_off() -> bool {
    std::env::var("HS_SWE_NET").as_deref() == Ok("off")
}

/// The host-side eval command, wrapped for the egress policy. With
/// `HS_SWE_NET=off` the f2p evaluator also loses network (unshare -n), so a
/// test suite cannot fetch reference material either.
#[must_use]
pub fn host_command_wrapper(command: &str) -> String {
    if egress_off() {
        format!("unshare -n sh -c {}", shell_quote(command))
    } else {
        command.to_string()
    }
}

fn shell_quote(c: &str) -> String {
    format!("'{}'", c.replace('\'', "'\\''"))
}

#[must_use]
pub fn run_host(ws: &Path, answer_path: &Path, command: &str, timeout_secs: u64) -> Value {
    let scratch = match prep(ws, answer_path) {
        Ok(Some(s)) => s,
        Ok(None) => unreachable!(),
        Err(early) => return early,
    };
    let out_f = scratch.join(".repexec-out");
    let err_f = scratch.join(".repexec-err");
    let wrapped = format!(
        "{} >'{}' 2>'{}'",
        host_command_wrapper(command.trim()),
        out_f.display(),
        err_f.display()
    );
    let child = Command::new("sh")
        .arg("-c")
        .arg(&wrapped)
        .current_dir(&scratch)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            cleanup_scratch(ws, &scratch, true);
            return json!({"$error": format!("spawn host exec: {e}")});
        }
    };
    let t0 = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(timeout_secs);
    let (status, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(st)) => break (Some(st), false),
            Ok(None) if t0.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break (None, true);
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(e) => {
                cleanup_scratch(ws, &scratch, true);
                return json!({"$error": format!("wait: {e}")});
            }
        }
    };
    let stdout = tail(&std::fs::read(&out_f).unwrap_or_default());
    let stderr = tail(&std::fs::read(&err_f).unwrap_or_default());
    cleanup_scratch(ws, &scratch, true);
    if timed_out {
        return json!({"applied": true, "timed_out": true, "timeout_secs": timeout_secs,
                      "stdout": stdout, "stderr": stderr});
    }
    json!({"applied": true, "timed_out": false,
           "exit_code": status.and_then(|s| s.code()).unwrap_or(-1),
           "stdout": stdout, "stderr": stderr})
}

/// Map a guardrail reason to its stable violation class - escalation and
/// telemetry count per class, not per exact command (the args change on
/// every retry; the class does not).
#[must_use]
pub fn violation_class(reason: &str) -> String {
    if reason.starts_with("git apply") {
        "git_apply".to_string()
    } else if reason.starts_with("raw diff-file write") {
        "diff_write".to_string()
    } else {
        "other".to_string()
    }
}

/// Pull the violation class back out of a gate result string (the loop
/// counts escalations from the `ToolCall` output, not from internals).
#[must_use]
pub fn extract_gate_class(output: &str) -> Option<String> {
    if !output.contains("forbidden edit path") {
        return None;
    }
    let pos = output.find("class: ")? + "class: ".len();
    let end = output[pos..]
        .find([',', ')', ' ', '\\', '"'])
        .map_or(output.len(), |i| pos + i);
    Some(output[pos..end].to_string())
}

/// Post-B8 escalation (B8: 6 same-class fires, the bare steer never
/// landed). The first fire speaks through the gate's own error; from the
/// SECOND same-class fire on, `record()` returns an escalating steer for
/// the loop to inject as feedback.
#[derive(Default)]
pub struct GuardrailEscalator {
    counts: std::collections::HashMap<String, usize>,
}

impl GuardrailEscalator {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// Count one fire of `class`; Some(steer) when it is a repeat.
    pub fn record(&mut self, class: &str) -> Option<String> {
        let n = {
            let c = self.counts.entry(class.to_string()).or_insert(0);
            *c += 1;
            *c
        };
        if n < 2 {
            return None;
        }
        Some(format!(
            "GUARDRAIL ESCALATION: {n} rejected edit-path attempts of class {class}. Repeating a rejected bypass cannot ever succeed - the CLASS is forbidden outright, not the specific command. Make the edit with edit.apply (search/replace blocks) and use repo.exec ONLY to build and test."
        ))
    }
}
