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


/// One search/replace block: replace `old` with `new` in the file at `path`.
#[derive(Clone, Debug)]
pub struct EditBlock {
    pub path: String,
    pub old: String,
    pub new: String,
}

/// Splice one block into content: exact-unique match first, then a
/// whitespace-tolerant fallback (trailing whitespace per line ignored) when
/// the exact match misses. Both modes require uniqueness.
fn splice_one(content: &str, old: &str, new: &str) -> Result<(String, &'static str), String> {
    let n = content.matches(old).count();
    if n == 1 {
        return Ok((content.replacen(old, new, 1), "exact"));
    }
    if n > 1 {
        return Err(format!(
            "old matches {n} times - add more surrounding context so it matches exactly once"
        ));
    }
    let old_lines: Vec<String> = old.lines().map(|l| l.trim_end().to_string()).collect();
    if old_lines.is_empty() {
        return Err("old not found in file".to_string());
    }
    let lines: Vec<&str> = content.split_inclusive("\n").collect();
    let mut starts = Vec::with_capacity(lines.len());
    let mut off = 0usize;
    for line in &lines {
        starts.push(off);
        off += line.len();
    }
    let mut hits = Vec::new();
    if lines.len() >= old_lines.len() {
        for w in 0..=(lines.len() - old_lines.len()) {
            let ok = old_lines
                .iter()
                .enumerate()
                .all(|(k, ol)| lines[w + k].trim_end() == ol.as_str());
            if ok {
                hits.push(w);
            }
        }
    }
    match hits.len() {
        0 => Err("old not found in file (tried exact and whitespace-tolerant match)".to_string()),
        1 => {
            let w = hits[0];
            let from = starts[w];
            let to = if w + old_lines.len() < starts.len() {
                starts[w + old_lines.len()]
            } else {
                content.len()
            };
            let mut out = String::with_capacity(content.len() + new.len());
            out.push_str(&content[..from]);
            out.push_str(new);
            out.push_str(&content[to..]);
            Ok((out, "fuzzy"))
        }
        m => Err(format!(
            "old matches {m} times under whitespace-tolerant matching - add more surrounding context"
        )),
    }
}

/// Apply search/replace blocks to the candidate (never the live ws). Blocks
/// apply in order; the call is all-or-nothing: any failing block returns
/// applied:false with per-block results and NOTHING is written, so earlier
/// candidate work survives intact. Model-facing edit path: no line numbers,
/// no diff syntax - the corrupt-patch failure class (measured 2026-09-05:
/// 6/6 model-written diffs failed on hunk-count arithmetic or truncated
/// tails) is designed out, not repaired.
pub fn apply_blocks(ws: &Path, blocks: &[EditBlock]) -> Value {
    if blocks.is_empty() {
        return json!({"applied": false, "error": "edits array is empty - pass at least one {path, old, new} block"});
    }
    let cand = match ensure_candidate(ws) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let mut staged: std::collections::BTreeMap<String, String> = Default::default();
    let mut results: Vec<Value> = Vec::new();
    for (i, b) in blocks.iter().enumerate() {
        if b.path.starts_with("/") || b.path.split('/').any(|s| s == "..") {
            return json!({"applied": false, "error": format!("block {i} ({}): path must be repo-relative, no absolute paths or ..", b.path), "results": results});
        }
        if b.old.is_empty() {
            return json!({"applied": false, "error": format!("block {i} ({}): old must be non-empty", b.path), "results": results});
        }
        if !staged.contains_key(&b.path) {
            match std::fs::read_to_string(cand.join(&b.path)) {
                Ok(c) => {
                    staged.insert(b.path.clone(), c);
                }
                Err(_) => {
                    return json!({"applied": false, "error": format!("block {i} ({}): file not found in candidate", b.path), "results": results});
                }
            }
        }
        let cur = staged[&b.path].clone();
        match splice_one(&cur, &b.old, &b.new) {
            Ok((next, how)) => {
                staged.insert(b.path.clone(), next);
                results.push(json!({"path": b.path, "status": "ok", "match": how}));
            }
            Err(e) => {
                results.push(json!({"path": b.path, "status": "failed", "detail": e}));
                return json!({"applied": false, "error": format!("block {i} ({}): {}", b.path, e), "results": results});
            }
        }
    }
    for (rel, content) in &staged {
        if let Err(e) = std::fs::write(cand.join(rel), content) {
            return json!({"$error": format!("write {rel}: {e}")});
        }
    }
    match read_cumulative(&cand) {
        Ok(cd) => {
            let files: Vec<&str> = cd
                .lines()
                .filter(|l| l.starts_with("diff --git"))
                .filter_map(|l| l.split(" b/").last())
                .collect();
            json!({"applied": true, "results": results, "cumulative_diff": cd, "files_changed": files})
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
