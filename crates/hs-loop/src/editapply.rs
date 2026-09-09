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

#[must_use]
pub fn candidate_dir(ws: &Path) -> PathBuf {
    let canon = ws.canonicalize().unwrap_or_else(|_| ws.to_path_buf());
    let mut h = DefaultHasher::new();
    h.write(canon.to_string_lossy().as_bytes());
    std::env::temp_dir().join(format!("editapply-{:016x}", h.finish()))
}

fn git(ws: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(ws)
        .output()
        .expect("git binary must be spawnable (harness hard dependency)")
}

fn ensure_candidate(ws: &Path) -> Result<PathBuf, Value> {
    let cand = candidate_dir(ws);
    if cand.join(".git").exists() {
        return Ok(cand);
    }
    // stale metadata from a prior crashed run must not block re-creation
    let _ = git(
        ws,
        &["worktree", "remove", "--force", &cand.to_string_lossy()],
    );
    let _ = std::fs::remove_dir_all(&cand);
    let out = git(
        ws,
        &[
            "worktree",
            "add",
            "--detach",
            &cand.to_string_lossy(),
            "HEAD",
        ],
    );
    if !out.status.success() {
        return Err(
            json!({"$error": format!("candidate worktree: {}", String::from_utf8_lossy(&out.stderr))}),
        );
    }
    Ok(cand)
}

fn read_cumulative(cand: &Path) -> Result<String, Value> {
    // .hs-eval.patch is harness eval machinery, never task content. Bases
    // contaminated by the pre-6398fc8f eval flow carry it COMMITTED in the
    // ws HEAD (conan-17302), so the candidate worktree tracks it: restore
    // the tracked copy (no-op + error when untracked at HEAD - ignored) and
    // exclude the path from staging and from the cumulative diff itself, so
    // neither the file nor its deletion can ever leak into the graded patch.
    let _ = git(cand, &["checkout", "HEAD", "--", ".hs-eval.patch"]);
    // .hs/ is the agent's blind-mode checks declaration: harness machinery,
    // like .hs-eval.patch - it must never join the submitted patch.
    let out = git(
        cand,
        &[
            "add",
            "-A",
            "--",
            ".",
            ":(exclude).hs-eval.patch",
            ":(exclude).hs",
        ],
    );
    if !out.status.success() {
        return Err(
            json!({"$error": format!("candidate stage: {}", String::from_utf8_lossy(&out.stderr))}),
        );
    }
    let diff = git(
        cand,
        &[
            "diff",
            "--cached",
            "HEAD",
            "--",
            ".",
            ":(exclude).hs-eval.patch",
            ":(exclude).hs",
        ],
    );
    if !diff.status.success() {
        return Err(
            json!({"$error": format!("candidate diff: {}", String::from_utf8_lossy(&diff.stderr))}),
        );
    }
    Ok(String::from_utf8_lossy(&diff.stdout).to_string())
}

/// Apply one incremental unified diff to the candidate. Returns applied +
/// `cumulative_diff` (+ `files_changed`) on success; clean feedback on a patch
/// that does not apply, leaving the candidate exactly as it was.
#[must_use]
pub fn apply(ws: &Path, diff: &str) -> Value {
    let Some(patch) = crate::repexec::extract_diff(diff) else {
        return json!({"applied": false, "note": "no unified diff in args.diff - pass one unified diff, raw or in a `diff` fence"});
    };
    let cand = match ensure_candidate(ws) {
        Ok(c) => c,
        Err(e) => return e,
    };
    match hs_bench::apply_model_patch(&cand, &patch) {
        Ok(hs_bench::ApplyResult::Applied) => {}
        Ok(hs_bench::ApplyResult::NoApply(msg)) => {
            return json!({"applied": false, "apply_error": msg});
        }
        Err(e) => {
            return json!({"$error": format!("apply machinery: {e:?}")});
        }
    }
    match read_cumulative(&cand) {
        Ok(cd) => {
            let files: Vec<&str> = cd
                .lines()
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
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
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
#[must_use]
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
        if b.path.starts_with('/') || b.path.split('/').any(|s| s == "..") {
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
#[must_use]
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
#[must_use]
pub fn reset(ws: &Path) -> Value {
    let cand = candidate_dir(ws);
    let _ = git(
        ws,
        &["worktree", "remove", "--force", &cand.to_string_lossy()],
    );
    let _ = std::fs::remove_dir_all(&cand);
    let _ = git(ws, &["worktree", "prune"]);
    json!({"ok": true})
}

// ─── Codex apply_patch edit path (2026-09-06) ───────────────────────
// The model never authors diff syntax: it edits the persistent candidate
// with the Codex apply_patch grammar (hs-applypatch crate, vendored from
// openai/codex via xai-org/grok-build) and answer.submit computes the
// final unified diff with git. Failure classes this kills by construction
// (session traces, 2026-09-06): corrupt hand-written hunks (8619/3314),
// empty fenced submissions (8609), prose-contaminated answer files.

/// Reject absolute paths and any `..` escape; the candidate root is the
/// only writable scope for model edits.
fn safe_join(base: &Path, rel: &Path) -> Result<PathBuf, Value> {
    if rel.is_absolute() {
        return Err(
            json!({"applied": false, "$error": format!("path must be repo-relative: {}", rel.display())}),
        );
    }
    for c in rel.components() {
        if !matches!(c, std::path::Component::Normal(_)) {
            return Err(
                json!({"applied": false, "$error": format!("path escapes the candidate: {}", rel.display())}),
            );
        }
    }
    Ok(base.join(rel))
}

enum Write {
    Set(PathBuf, String),
    Del(PathBuf),
}

/// Apply one Codex-grammar patch to the candidate worktree. Atomic: every
/// hunk is validated (paths, existence, context match) BEFORE any write,
/// so a failure leaves the candidate byte-identical. Returns applied +
/// `cumulative_diff` on success; a named $error on failure.
#[must_use]
pub fn apply_codex_patch(ws: &Path, patch_text: &str) -> Value {
    use hs_applypatch::parser::Hunk;
    let parsed = match hs_applypatch::parser::parse_patch(patch_text) {
        Ok(p) => p,
        Err(e) => return json!({"applied": false, "$error": format!("patch parse: {e}")}),
    };
    let cand = match ensure_candidate(ws) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let mut writes: Vec<Write> = Vec::new();
    let mut moves: Vec<(PathBuf, PathBuf)> = Vec::new();
    for h in &parsed.hunks {
        match h {
            Hunk::AddFile { path, contents } => {
                let dest = match safe_join(&cand, path) {
                    Ok(d) => d,
                    Err(e) => return e,
                };
                if dest.exists() {
                    return json!({"applied": false, "$error": format!("{}: already exists in the candidate - use Update File", path.display())});
                }
                writes.push(Write::Set(dest, contents.clone()));
            }
            Hunk::DeleteFile { path } => {
                let dest = match safe_join(&cand, path) {
                    Ok(d) => d,
                    Err(e) => return e,
                };
                if !dest.exists() {
                    return json!({"applied": false, "$error": format!("{}: no such file in the candidate", path.display())});
                }
                writes.push(Write::Del(dest));
            }
            Hunk::UpdateFile {
                path,
                move_path,
                chunks,
            } => {
                let src = match safe_join(&cand, path) {
                    Ok(d) => d,
                    Err(e) => return e,
                };
                let original = match std::fs::read_to_string(&src) {
                    Ok(s) => s,
                    Err(_) => {
                        return json!({"applied": false, "$error": format!("{}: no such file in the candidate", path.display())})
                    }
                };
                let new = match hs_applypatch::apply::derive_new_contents(&original, path, chunks) {
                    Ok(n) => n,
                    Err(e) => {
                        return json!({"applied": false, "$error": format!("{}: {e} - context must match the CURRENT candidate exactly (repo.read it again, or op=diff to see your cumulative state)", path.display())})
                    }
                };
                if let Some(mp) = move_path {
                    match safe_join(&cand, mp) {
                        Ok(dst) => moves.push((src.clone(), dst)),
                        Err(e) => return e,
                    }
                }
                writes.push(Write::Set(src, new));
            }
        }
    }
    for w in writes {
        match w {
            Write::Set(p, contents) => {
                if let Some(parent) = p.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        return json!({"applied": false, "$error": format!("mkdir {}: {e}", parent.display())});
                    }
                }
                if let Err(e) = std::fs::write(&p, contents) {
                    return json!({"applied": false, "$error": format!("write {}: {e}", p.display())});
                }
            }
            Write::Del(p) => {
                if let Err(e) = std::fs::remove_file(&p) {
                    return json!({"applied": false, "$error": format!("delete {}: {e}", p.display())});
                }
            }
        }
    }
    for (src, dst) in moves {
        if src != dst {
            if let Some(parent) = dst.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::rename(&src, &dst) {
                return json!({"applied": false, "$error": format!("move {} -> {}: {e}", src.display(), dst.display())});
            }
        }
    }
    match read_cumulative(&cand) {
        Ok(diff) => json!({"applied": true, "cumulative_diff": diff}),
        Err(e) => e,
    }
}

/// answer.submit backend: the answer file is the candidate's cumulative
/// diff, computed with git - never model-authored text. No candidate or an
/// untouched candidate is a steering error, not a submission (replay class:
/// 8609's literal empty `diff` fence, 2026-09-06).
#[must_use]
pub fn answer_submit(ws: &Path, answer_path: &Path) -> Value {
    let cand = candidate_dir(ws);
    let diff = if cand.join(".git").exists() {
        match read_cumulative(&cand) {
            Ok(d) => d,
            Err(e) => return e,
        }
    } else {
        String::new()
    };
    if diff.trim().is_empty() {
        return json!({"$error": "nothing to submit: the candidate has no edits - make your fix with edit.patch first, verify it with repo.exec, then answer.submit"});
    }
    match std::fs::write(answer_path, &diff) {
        Ok(()) => {
            json!({"written": true, "path": answer_path.to_string_lossy(), "bytes": diff.len()})
        }
        Err(e) => json!({"$error": e.to_string()}),
    }
}

// ─── Hashline anchor edit path (2026-09-06, bake-off arm) ───────────
// Grok Build's hashline flavor, hairspring-native: repo.read shows
// LINE:HASH prefixes (anchored_read), edit.anchor applies anchor-typed
// ops validated against the pre-edit snapshot (hs-hashline engine,
// vendored). Same candidate-worktree + computed-submit invariants as
// edit.patch.

/// Render file content with LINE:HASH anchors (Grok chunk scheme,
/// `hash_len=3`, `chunk_size=8` - their shipped default).
#[must_use]
pub fn anchored_read(content: &str) -> String {
    let scheme = hs_hashline::config::HashlineSchemeParams::default()
        .build_scheme()
        .expect("default scheme builds");
    hs_hashline::render::format_hashline_content(content, None, None, &*scheme).0
}

/// Apply anchor-typed ops to a candidate file. Anchors are validated
/// against the CURRENT candidate content; a stale or wrong anchor is a
/// named error and the file stays byte-identical (engine applies
/// bottom-up after full validation). Returns snippet with FRESH anchors
/// plus the cumulative diff on success.
#[must_use]
pub fn apply_anchor_edits(ws: &Path, path: &str, edits: Value) -> Value {
    use hs_hashline::edit::apply::apply_edits;
    use hs_hashline::edit::types::{HashlineEditOutput, HashlineOp};
    let rel = Path::new(path);
    let ops: Vec<HashlineOp> = match serde_json::from_value(edits.clone()) {
        Ok(v) => v,
        Err(_) => match edits.as_str().and_then(|s| serde_json::from_str(s).ok()) {
            Some(v) => v,
            None => {
                return json!({"applied": false, "$error": "edits must be an array of {op, anchor, content} operations"})
            }
        },
    };
    let cand = match ensure_candidate(ws) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let dest = match safe_join(&cand, rel) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let exists = dest.exists();
    let content = if exists {
        match std::fs::read_to_string(&dest) {
            Ok(s) => s,
            Err(e) => return json!({"applied": false, "$error": format!("read {path}: {e}")}),
        }
    } else {
        // only a bare `write` op may create a file
        if ops.len() == 1 && matches!(ops[0], HashlineOp::Write { .. }) {
            String::new()
        } else {
            return json!({"applied": false, "$error": format!("{path}: no such file in the candidate (a single write op creates new files)")});
        }
    };
    let scheme = match hs_hashline::config::HashlineSchemeParams::default().build_scheme() {
        Ok(s) => s,
        Err(e) => return json!({"applied": false, "$error": e}),
    };
    let result = apply_edits(&content, &ops, rel, &*scheme);
    match result.output {
        HashlineEditOutput::EditsApplied(applied) => {
            let new_content = match result.new_content {
                Some(n) => n,
                None => return json!({"applied": false, "$error": "engine returned no content"}),
            };
            if let Some(parent) = dest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(&dest, new_content) {
                return json!({"applied": false, "$error": format!("write {path}: {e}")});
            }
            match read_cumulative(&cand) {
                Ok(diff) => json!({
                    "applied": true,
                    "ops": applied.applied,
                    "scheme": applied.scheme,
                    "snippet": applied.snippet,
                    "snippet_start_line": applied.snippet_start_line,
                    "warnings": applied.warnings,
                    "cumulative_diff": diff,
                }),
                Err(e) => e,
            }
        }
        HashlineEditOutput::Error(err) => json!({
            "applied": false,
            "$error": format!("anchor validation failed for {path}: {}", serde_json::to_string_pretty(&err).unwrap_or_default()),
        }),
    }
}

/// repo.read in anchor mode reads the CANDIDATE (the model's edits change
/// anchors; reading the pristine base would make every anchor stale after
/// the first edit). Same windowing contract as `repotools::read_repo_window`,
/// content rendered with LINE:HASH prefixes.
pub fn anchored_read_window(
    ws: &Path,
    rel: &str,
    start_line: Option<u64>,
    max_lines: Option<u64>,
) -> Result<Value, String> {
    let cand = ensure_candidate(ws).map_err(|e| e.to_string())?;
    let dest = safe_join(&cand, Path::new(rel)).map_err(|e| e.to_string())?;
    let meta = std::fs::metadata(&dest).map_err(|_| format!("not found: {rel}"))?;
    if meta.is_dir() {
        return Err(format!("is a directory: {rel}"));
    }
    let text = std::fs::read_to_string(&dest)
        .map_err(|e| format!("not found or not utf-8: {rel} ({e})"))?;
    let total_lines = text.lines().count() as u64;
    let start = start_line.unwrap_or(1).max(1);
    let want = max_lines.unwrap_or(400).min(400);
    let scheme = hs_hashline::config::HashlineSchemeParams::default()
        .build_scheme()
        .map_err(|e| e.clone())?;
    let (anchored, _raw) = hs_hashline::render::format_hashline_content(
        &text,
        Some(start as usize),
        Some(want as usize),
        &*scheme,
    );
    let end = (start - 1 + want).min(total_lines);
    Ok(json!({
        "path": rel,
        "content": anchored,
        "start_line": start,
        "end_line": end,
        "total_lines": total_lines,
        "truncated": end < total_lines,
        "anchor_scheme": "chunk h=3 c=8",
    }))
}
