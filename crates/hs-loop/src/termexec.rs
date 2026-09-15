//! Terminal-bench mission tool "term.exec": run a command DIRECTLY in the
//! mission workdir. No per-call filesystem copy: files, installs, and
//! services persist between calls, which is exactly the machine state
//! terminal-bench grades after the agent finishes. Hard timeout enforced
//! (kill on expiry).
//!
//! Confinement (mission isolation, 2026-09-10): when the harness exports
//! HS_PROJECT_ROOT (hs-repl always does - `--project-dir` or the session
//! work area by default) the command runs inside a bwrap namespace where
//! the project root is the ONLY host filesystem bound read-write: /usr,
//! /etc and the /bin|/lib symlinks are read-only binds, /tmp is a fresh
//! tmpfs scratch, the environment is scrubbed to a minimal PATH/HOME (no
//! harness HS_* config, no provider keys), and the network stays up -
//! package installs are part of the terminal-bench contract. Hostile
//! absolute reads and `..` escapes find nothing because nothing outside
//! the root is mounted. The old "the container is the sandbox" assumption
//! held on the one-container-per-task bench rig; in shared-box REPL mode
//! it let missions read every harness file on the host. Fail-closed: no
//! bwrap, no run.
//!
//! macOS backend (2026-09-10, Eric: missions are first-class on macOS):
//! the same contract is enforced with the kernel Seatbelt sandbox via
//! sandbox-exec - the mechanism Bazel, Nix, Homebrew, and Claude Code
//! rely on, functional on macOS 15. The generated profile is
//! `(allow default)` (network stays up, same ruling) + `deny file-write*`
//! everywhere + later, higher-precedence `allow file-write*` under the
//! project root, /private/tmp, /private/var/folders and /dev/null: the
//! root is the ONLY host filesystem writable, the environment is scrubbed
//! to the same minimal PATH/HOME, and no sandbox-exec means no run.
//! `run_readonly` parity: instead of a uid drop (no root on a stranger's
//! Mac) the verifier profile OMITS the root from the writable set - task
//! files are read-only BY MECHANISM, same guarantee, different lever.
//!
//! Two surfaces: `run` for the AUTHORING agent (root inside the task
//! container - the agent is supposed to mutate), `run_readonly` for the
//! VERIFIER (the independent critic): the project root is bound READ-ONLY
//! (Linux) or omitted from the profile's writable set (macOS), so task
//! files are read-only BY MECHANISM. (The Linux uid/gid `nobody` drop
//! remains as defense-in-depth but is NOT the enforcement: bwrap maps the
//! inner uid to the outer euid - hostile finding 2026-09-10.) Fail-closed:
//! a non-root caller on the legacy path cannot drop privileges, so the
//! call refuses instead of running unenforced.

use serde_json::Value;
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::time::{Duration, Instant};

fn tail(s: String, n: usize) -> String {
    crate::msgfmt::tail_bytes_safe(&s, n)
}

/// uid/gid of the verifier's unprivileged identity.
const NOBODY: u32 = 65534;

/// Effective uid from /proc (dependency-free geteuid). Unknown reads as
/// NOT root, which is the fail-closed answer for `run_readonly`.
fn euid() -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(2))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(u32::MAX)
}

/// Shared tail of both surfaces: own process group, hard timeout that
/// kills the whole tree, bounded output.
fn collect(mut child: std::process::Child, timeout_secs: u64) -> Value {
    // The command runs in its OWN process group: a timeout must kill the
    // whole tree. Killing only the wrapper leaves orphaned grandchildren
    // holding the stdout/stderr pipes open, and wait_with_output then
    // blocks until THEY exit (observed live 2026-09-07: `grep -r X /`
    // orphaned by a timed-out call burned 30 min per call at 99% CPU while
    // the mission thread sat in wait_with_output).
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let timed_out = loop {
        match child.try_wait() {
            Ok(Some(_)) => break false,
            Ok(None) => {
                if Instant::now() >= deadline {
                    // Group kill first (grandchildren release the pipes),
                    // then the direct child as belt-and-braces. bash's
                    // builtin kill keeps this dependency-free.
                    let pgid = child.id();
                    let _ = std::process::Command::new("bash")
                        .args(["-c", &format!("kill -KILL -- -{pgid} 2>/dev/null")])
                        .status();
                    let _ = child.kill();
                    let _ = child.wait();
                    break true;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return serde_json::json!({"$error": format!("wait: {e}")}),
        }
    };
    let out = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => return serde_json::json!({"$error": format!("output: {e}")}),
    };
    serde_json::json!({
        "exit_code": out.status.code().unwrap_or(-1),
        "timed_out": timed_out,
        "stdout": tail(String::from_utf8_lossy(&out.stdout).into_owned(), 6000),
        "stderr": tail(String::from_utf8_lossy(&out.stderr).into_owned(), 3000),
    })
}

/// Startup probe for the confinement mechanism (hs-repl runs this once
/// before any mission). The bwrap binary being present is not the whole
/// story - blocked unprivileged userns (a sysctl, container policy)
/// spawns fine and fails inside the namespace - so the probe runs the
/// real namespace shape with `true`. Either failure must surface HERE,
/// at startup with the install hint, not as a mid-mission burn of tool
/// refusals (observed 2026-09-10: a bwrap-less first run looped
/// "bwrap sandbox unavailable" to steps_exhausted).
#[cfg(target_os = "macos")]
pub fn sandbox_probe() -> Result<(), String> {
    let out = std::process::Command::new("sandbox-exec")
        .arg("-p")
        .arg("(version 1)(allow default)")
        .arg("/usr/bin/true")
        .env_clear()
        .output();
    match out {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(format!(
            "missions need the macOS Seatbelt sandbox: the sandbox-exec probe \
             exited {status} ({stderr}). sandbox-exec ships with macOS - this \
             system cannot confine missions, and an unconfined run is never \
             the fallback.",
            status = o.status,
            stderr = String::from_utf8_lossy(&o.stderr).trim()
        )),
        Err(e) => Err(format!(
            "missions need the macOS Seatbelt sandbox: sandbox-exec not found \
             ({e}). It ships with macOS (the mechanism Bazel, Homebrew, and \
             Claude Code use) - its absence means a stripped system; refusing \
             to run unconfined."
        )),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn sandbox_probe() -> Result<(), String> {
    let tmp = std::env::temp_dir();
    let work = tmp.canonicalize().unwrap_or(tmp);
    let mut child = spawn_confined(&work, &work, "true", None)
        .map_err(|e| sandbox_hint(e["$error"].as_str().unwrap_or("spawn failed").to_string()))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(sandbox_hint("probe timed out".to_string()));
            }
            Err(e) => return Err(sandbox_hint(e.to_string())),
        }
    };
    if status.success() {
        return Ok(());
    }
    // The probe's own stderr carries the actionable cause (Ubuntu 24.04
    // AppArmor userns denial, container policy, ...) - a bare exit code
    // leaves the stranger (and CI) guessing.
    let mut probe_err = String::new();
    if let Some(mut se) = child.stderr.take() {
        use std::io::Read as _;
        let _ = se.read_to_string(&mut probe_err);
    }
    Err(sandbox_hint(format!(
        "probe exited {status}: {}",
        probe_err.trim()
    )))
}

// Linux-only like its only caller (the not-macos probe above): without
// the gate, macOS builds compile it unused and warn (Eric's fresh-install
// report 2026-09-10). The macOS probe carries its own Seatbelt hint.
#[cfg(not(target_os = "macos"))]
fn sandbox_hint(detail: String) -> String {
    format!(
        "missions need the bubblewrap sandbox ({detail}). Every tool call \
         is confined by mechanism and never falls back to an unconfined \
         run. Install \
         it - Debian/Ubuntu: apt install bubblewrap; Fedora: dnf install \
         bubblewrap; Arch: pacman -S bubblewrap. Inside a container, allow \
         unprivileged user namespaces; on Ubuntu 24.04+ they are \
         AppArmor-restricted by default: sudo sysctl -w \
         kernel.apparmor_restrict_unprivileged_userns=0. (macOS confines \
         with the Seatbelt backend instead - this check is Linux-only.)"
    )
}

/// Escape a path for embedding in an SBPL string literal. Compiled on
/// every platform: the profile shape is unit-tested from Linux.
fn sbpl_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The Seatbelt profile for a mission exec. `root` is the session work
/// root; `writable` selects the surface: the AUTHORING agent gets `root`
/// as the only writable host tree, the VERIFIER gets NO writable root -
/// and because a session root can sit INSIDE a scratch tree (/tmp/hs-demo
/// resolves to /private/tmp; tempdir roots land in /private/var/folders),
/// the verifier profile ends with a LATER deny for the root itself.
/// Seatbelt precedence is "last match wins", so the trailing deny
/// overrides the scratch allows for the submission only (hostile CI
/// finding 2026-09-10: the scratch-only profile let the verifier write
/// a submission rooted under the scratch tree). `(allow default)` keeps
/// the network up (terminal-bench ruling) and exec/mach basics working.
/// Compiled on every platform: the profile shape is unit-tested from
/// Linux.
pub fn seatbelt_profile(root: &std::path::Path, writable: bool) -> String {
    let root_e = sbpl_escape(&root.to_string_lossy());
    let mut allows = String::new();
    if writable {
        allows.push_str(&format!("    (subpath \"{root_e}\")\n"));
    }
    // /tmp and /var are symlinks; Seatbelt matches on resolved paths.
    allows.push_str("    (subpath \"/private/tmp\")\n");
    allows.push_str("    (subpath \"/private/var/folders\")\n");
    allows.push_str("    (literal \"/dev/null\")\n");
    allows.push_str("    (literal \"/dev/tty\")\n");
    let mut profile = format!(
        "(version 1)\n(allow default)\n(deny file-write* (regex \".*\"))\n(allow file-write*\n{allows})\n"
    );
    if !writable {
        profile.push_str(&format!("(deny file-write* (subpath \"{root_e}\"))\n"));
    }
    profile
}

/// Confined spawn: see the module doc. On macOS `identity` (uid/gid
/// nobody for the verifier) is honored BY THE PROFILE instead: dropping
/// to nobody needs root, which a stranger's Mac does not grant, and the
/// read-only-by-mechanism guarantee comes from the writable set.
#[cfg(target_os = "macos")]
fn spawn_confined(
    root: &std::path::Path,
    workdir: &std::path::Path,
    command: &str,
    identity: Option<(u32, u32)>,
) -> Result<std::process::Child, Value> {
    let profile = seatbelt_profile(root, identity.is_none());
    let mut cmd = std::process::Command::new("sandbox-exec");
    cmd.arg("-p")
        .arg(&profile)
        .arg("/bin/bash")
        .arg("-c")
        .arg(command)
        .current_dir(workdir)
        .env_clear()
        .env(
            "PATH",
            if identity.is_none() {
                format!(
                    "{}/.cargo/bin:{}/.local/bin:/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin",
                    root.display(),
                    root.display()
                )
            } else {
                "/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin".to_string()
            },
        )
        .env(
            "HOME",
            if identity.is_none() {
                root.as_os_str()
            } else {
                std::ffi::OsStr::new("/tmp")
            },
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    cmd.spawn().map_err(|e| {
        serde_json::json!({"$error": format!(
            "seatbelt sandbox unavailable ({e}) - refusing to run unconfined"
        )})
    })
}

/// Confined spawn: see the module doc. `identity` drops to uid/gid
/// nobody for the verifier surface. The bwrap binary missing is a
/// fail-CLOSED error - an unconfined run is never the fallback.
#[cfg(not(target_os = "macos"))]
fn spawn_confined(
    root: &std::path::Path,
    workdir: &std::path::Path,
    command: &str,
    identity: Option<(u32, u32)>,
) -> Result<std::process::Child, Value> {
    let root_s = root.to_string_lossy().into_owned();
    let mut cmd = std::process::Command::new("bwrap");
    let author_path =
        format!("{root_s}/.cargo/bin:{root_s}/.local/bin:/usr/local/bin:/usr/bin:/bin");
    cmd.args([
        "--unshare-user",
        "--unshare-pid",
        "--unshare-uts",
        "--unshare-ipc",
        "--unshare-cgroup",
        "--die-with-parent",
        "--clearenv",
        "--setenv",
        "PATH",
        if identity.is_none() {
            &author_path
        } else {
            "/usr/local/bin:/usr/bin:/bin"
        },
        "--setenv",
        "HOME",
        if identity.is_none() { &root_s } else { "/tmp" },
        "--ro-bind",
        "/usr",
        "/usr",
        "--symlink",
        "usr/bin",
        "/bin",
        "--symlink",
        "usr/lib",
        "/lib",
        "--symlink",
        "usr/lib64",
        "/lib64",
        "--ro-bind",
        "/etc",
        "/etc",
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--tmpfs",
        "/tmp",
    ]);
    // DNS: /etc/resolv.conf commonly symlinks into /run (systemd-resolved)
    // and /run is never bound, so every lookup dies inside. Copy the
    // RESOLVED contents to scratch and ro-bind at the canonical path
    // (same defect + fix as repexec, 2026-09-10).
    if let Ok(target) = std::fs::canonicalize("/etc/resolv.conf") {
        if target != std::path::Path::new("/etc/resolv.conf") {
            if let Ok(bytes) = std::fs::read(&target) {
                let scratch_resolv = std::env::temp_dir()
                    .join(format!(".hs-termexec-resolv-{}", std::process::id()));
                if std::fs::write(&scratch_resolv, bytes).is_ok() {
                    cmd.arg("--ro-bind")
                        .arg(&scratch_resolv)
                        .arg(target.display().to_string());
                }
            }
        }
    }
    // The VERIFIER's root is READ-ONLY BY MECHANISM: --ro-bind, not the
    // uid drop. Hostile finding 2026-09-10: under --unshare-user bwrap
    // maps the requested inner uid to the OUTER euid (uid_map
    // "65534 0 1" on a root host), so the inner-nobody drop alone let
    // the critic write root-owned task files. The uid args stay as
    // defense-in-depth; the bind mode is the enforcement.
    let bind = if identity.is_some() {
        "--ro-bind"
    } else {
        "--bind"
    };
    cmd.args([bind, &root_s, &root_s]);
    if let Some((uid, gid)) = identity {
        cmd.args(["--uid", &uid.to_string(), "--gid", &gid.to_string()]);
    }
    cmd.args(["--chdir", &workdir.to_string_lossy(), "bash", "-c", command])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    cmd.spawn().map_err(|e| {
        serde_json::json!({"$error": format!(
            "bwrap sandbox unavailable ({e}) - refusing to run unconfined"
        )})
    })
}

#[must_use]
pub fn run(workdir: &std::path::Path, command: &str, timeout_secs: u64) -> Value {
    if command.trim().is_empty() {
        return serde_json::json!({"$error": "pass command: a bash command line"});
    }
    if let Some(root) = crate::projectroot::project_root() {
        let workdir = match crate::projectroot::confine_existing(workdir, "term.exec workdir") {
            Ok(w) => w,
            Err(e) => return e,
        };
        let child = match spawn_confined(&root, &workdir, command, None) {
            Ok(c) => c,
            Err(e) => return e,
        };
        return collect(child, timeout_secs);
    }
    let child = match std::process::Command::new("bash")
        .args(["-c", command])
        .current_dir(workdir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return serde_json::json!({"$error": format!("spawn: {e}")}),
    };
    collect(child, timeout_secs)
}

/// The verifier's surface: identical discipline to `run`, but the command
/// executes as uid/gid `nobody` with the supplementary group list cleared,
/// so it can read world-readable task state and write /tmp scratch - and
/// CANNOT modify, move, or delete root-owned task files, whatever the
/// (model-authored) command line says. Enforcement is positional, not a
/// prompt promise.
#[must_use]
pub fn run_readonly(workdir: &std::path::Path, command: &str, timeout_secs: u64) -> Value {
    if command.trim().is_empty() {
        return serde_json::json!({"$error": "pass command: a bash command line"});
    }
    if let Some(root) = crate::projectroot::project_root() {
        // Same confinement as `run`, but the project root is bound
        // READ-ONLY (--ro-bind in spawn_confined): the verifier's
        // commands are model-authored too, and task files stay
        // unmodifiable BY MECHANISM no matter how the userns maps uids
        // (bwrap maps inner nobody to the outer euid - the 2026-09-10
        // hostile finding). Scratch in /tmp persists between calls.
        let workdir = match crate::projectroot::confine_existing(workdir, "verify workdir") {
            Ok(w) => w,
            Err(e) => return e,
        };
        let child = match spawn_confined(&root, &workdir, command, Some((NOBODY, NOBODY))) {
            Ok(c) => c,
            Err(e) => return e,
        };
        return collect(child, timeout_secs);
    }
    if euid() != 0 {
        return serde_json::json!({"$error": "run_readonly requires root to drop privileges (setuid nobody); refusing to run unenforced"});
    }
    let mut cmd = std::process::Command::new("bash");
    cmd.args(["-c", command])
        .current_dir(workdir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .env("HOME", "/tmp")
        .gid(NOBODY)
        .uid(NOBODY);
    // std's uid/gid setup also clears the SUPPLEMENTARY group list
    // (setgroups(0, NULL) between setgid and setuid - verified by strace
    // on this toolchain), so group-0 write bits do not survive the drop.
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return serde_json::json!({"$error": format!("spawn: {e}")}),
    };
    collect(child, timeout_secs)
}
