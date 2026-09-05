//! edit.apply backend: a persistent candidate worktree per base workspace.
//! The model applies incremental unified diffs to the CANDIDATE (never the
//! live ws); every call returns the cumulative diff vs base so the submit
//! path grades exactly what the model built - no re-serialization, no lost
//! intermediate state (plugin processes are ephemeral; state lives on disk).

use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;
use std::path::{Path, PathBuf};
use std::process::Command;

fn candidate_dir(ws: &Path) -> PathBuf {
    let canon = ws.canonicalize().unwrap_or_else(|_| ws.to_path_buf());
    let mut h = DefaultHasher::new();
    h.write(canon.to_string_lossy().as_bytes());
    std::env::temp_dir().join(format!("editapply-{:016x}", h.finish()))
}

fn git(ws: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git").args(args).current_dir(ws).output()
        .unwrap_or_else(|e| panic!("git spawn: {e}"))
}

fn ensure_candidate(ws: &Path) -> Result<PathBuf, Value> {
    let cand = candidate_dir(ws);
    if cand.join(".git").exists() {
        return Ok(cand);
    }
    // stale metadata from a prior crashed run must not block re-creation
    let _ = git(ws, &["worktree", "remove", "--force", &cand.to_string_lossy()]);
    let _ = std::fs::remove_dir_all(&cand);
    let out = git(ws, &["worktree", "add", "--detach", &cand.to_string_lossy(), "HEAD"]);
    if !out.status.success() {
        return Err(json!({"$error": format!("candidate worktree: {}", String::from_utf8_lossy(&out.stderr))}));
    }
    Ok(cand)
}

fn read_cumulative(cand: &Path) -> Result<String, Value> {
    // the apply helper drops .bench-model.patch in the tree; never stage it
    let _ = std::fs::remove_file(cand.join(".bench-model.patch"));
    let out = git(cand, &["add", "-A"]);
    if !out.status.success() {
        return Err(json!({"$error": format!("candidate stage: {}", String::from_utf8_lossy(&out.stderr))}));
    }
    let diff = git(cand, &["diff", "--cached", "HEAD"]);
    if !diff.status.success() {
        return Err(json!({"$error": format!("candidate diff: {}", String::from_utf8_lossy(&diff.stderr))}));
    }
    Ok(String::from_utf8_lossy(&diff.stdout).to_string())
}

/// Apply one incremental unified diff to the candidate. Returns applied +
/// cumulative_diff (+ files_changed) on success; clean feedback on a patch
/// that does not apply, leaving the candidate exactly as it was.
pub fn apply(ws: &Path, diff: &str) -> Value {
    let Some(patch) = crate::repexec::extract_diff(diff) else {
        return json!({"applied": false, "note": "no unified diff in args.diff - pass one unified diff, raw or in a ```diff fence"});
    };
    let cand = match ensure_candidate(ws) {
        Ok(c) => c,
        Err(e) => return e,
    };
    match hs_bench::apply_model_patch(&cand, &patch) {
        Ok(hs_bench::ApplyResult::Applied) => {}
        Ok(hs_bench::ApplyResult::NoApply(msg)) => {
            let _ = std::fs::remove_file(cand.join(".bench-model.patch"));
            return json!({"applied": false, "apply_error": msg});
        }
        Err(e) => {
            let _ = std::fs::remove_file(cand.join(".bench-model.patch"));
            return json!({"$error": format!("apply machinery: {e:?}")});
        }
    }
    let _ = std::fs::remove_file(cand.join(".bench-model.patch"));
    match read_cumulative(&cand) {
        Ok(cd) => {
            let files: Vec<&str> = cd.lines()
                .filter(|l| l.starts_with("diff --git"))
                .filter_map(|l| l.split(" b/").last())
                .collect();
            json!({"applied": true, "cumulative_diff": cd, "files_changed": files})
        }
        Err(e) => e,
    }
}

/// The cumulative diff without applying anything new.
pub fn cumulative_diff(ws: &Path) -> Value {
    let cand = candidate_dir(ws);
    if !cand.join(".git").exists() {
        return json!({"has_candidate": false, "cumulative_diff": ""});
    }
    match read_cumulative(&cand) {
        Ok(cd) => json!({"has_candidate": true, "cumulative_diff": cd}),
        Err(e) => e,
    }
}

/// Discard the candidate (fresh start) and prune worktree metadata.
pub fn reset(ws: &Path) -> Value {
    let cand = candidate_dir(ws);
    let _ = git(ws, &["worktree", "remove", "--force", &cand.to_string_lossy()]);
    let _ = std::fs::remove_dir_all(&cand);
    let _ = git(ws, &["worktree", "prune"]);
    json!({"ok": true})
}
