//! repo.read / repo.search tool cores for SWE missions: read-only,
//! workspace-sandboxed repo access. A model that cannot see the repo is
//! patching blind (observed 2026-09-04: plausible hunk, guessed line
//! numbers, patch does not apply). These tools are the faithful surface.

use serde_json::json;
use std::path::{Component, Path, PathBuf};

/// Max bytes of file content returned by repo.read.
pub const READ_CAP: usize = 40_000;
/// Max match rows returned by repo.search.
pub const SEARCH_CAP: usize = 100;
/// Max file size search will open (skips giants, e.g. vendored blobs).
pub const SEARCH_FILE_CAP: u64 = 1_000_000;

/// Resolve `rel` inside `ws`, rejecting any escape (.., absolute, symlink
/// pointing outside). Returns the canonical path on success.
fn resolve_inside(ws: &Path, rel: &str) -> Result<PathBuf, String> {
    if rel.is_empty() {
        return Err("empty path".to_string());
    }
    let p = Path::new(rel);
    if p.is_absolute() {
        return Err(format!("escape: absolute path not allowed: {rel}"));
    }
    let mut clean = ws.to_path_buf();
    for c in p.components() {
        match c {
            Component::Normal(seg) => clean.push(seg),
            Component::CurDir => {}
            Component::ParentDir => {
                // A ".." that would leave the workspace is an escape.
                if clean == ws {
                    return Err(format!("escape: {rel}"));
                }
                clean.pop();
            }
            _ => return Err(format!("escape: bad component in {rel}")),
        }
    }
    // Canonicalize to defeat symlinks; the result must stay under ws.
    let canon = clean
        .canonicalize()
        .map_err(|_| format!("not found: {rel}"))?;
    let ws_canon = ws
        .canonicalize()
        .map_err(|e| format!("workspace broken: {e}"))?;
    if !canon.starts_with(&ws_canon) {
        return Err(format!("escape: {rel} resolves outside the workspace"));
    }
    Ok(canon)
}

pub fn read_repo_file(ws: &Path, rel: &str) -> Result<serde_json::Value, String> {
    let canon = resolve_inside(ws, rel)?;
    let meta = std::fs::metadata(&canon).map_err(|_| format!("not found: {rel}"))?;
    if meta.is_dir() {
        return Err(format!("is a directory: {rel}"));
    }
    let bytes = std::fs::read(&canon).map_err(|e| format!("not found: {rel} ({e})"))?;
    let total = bytes.len() as u64;
    let truncated = bytes.len() > READ_CAP;
    let slice = if truncated { &bytes[..READ_CAP] } else { &bytes[..] };
    Ok(json!({
        "path": rel,
        "content": String::from_utf8_lossy(slice),
        "truncated": truncated,
        "total_bytes": total,
    }))
}

pub fn search_repo(ws: &Path, pattern: &str) -> Result<serde_json::Value, String> {
    if pattern.is_empty() {
        return Err("empty pattern".to_string());
    }
    let ws_canon = ws.canonicalize().map_err(|e| format!("workspace broken: {e}"))?;
    let mut matches = Vec::new();
    let mut capped = false;
    let mut stack = vec![ws_canon.clone()];
    'walk: while let Some(dir) = stack.pop() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for ent in rd.flatten() {
            let path = ent.path();
            let name = ent.file_name();
            let name = name.to_string_lossy();
            if name == ".git" || name == "target" || name == "node_modules" {
                continue;
            }
            let ft = match ent.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_dir() {
                stack.push(path);
                continue;
            }
            if ft.is_symlink() {
                // Stay inside: only follow links that resolve under ws.
                match path.canonicalize() {
                    Ok(c) if c.starts_with(&ws_canon) => {}
                    _ => continue,
                }
            }
            let size = ent.metadata().map(|m| m.len()).unwrap_or(0);
            if size > SEARCH_FILE_CAP {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue; // binary or unreadable
            };
            let rel = path
                .strip_prefix(&ws_canon)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| name.to_string());
            for (i, line) in text.lines().enumerate() {
                if line.contains(pattern) {
                    matches.push(json!({
                        "path": rel,
                        "line": i + 1,
                        "text": line.trim(),
                    }));
                    if matches.len() >= SEARCH_CAP {
                        capped = true;
                        break 'walk;
                    }
                }
            }
        }
    }
    Ok(json!({"pattern": pattern, "matches": matches, "capped": capped}))
}
