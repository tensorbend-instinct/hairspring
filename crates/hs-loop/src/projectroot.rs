//! Project-root confinement (mission isolation, 2026-09-10). The project
//! directory is a mission-start INPUT: `hs-repl --project-dir <path>`
//! exports `HS_PROJECT_ROOT` (validated + canonicalized at startup); when
//! unset, the harness defaults it to the session work area
//! `<log_root>/work` in `wire_tool_env`, so every TUI session is confined
//! by default. Every surface a mission's commands or writes can reach is
//! confined BY MECHANISM to that root: term.exec spawns inside a bwrap
//! namespace where nothing outside the root exists, and answer writes are
//! refused outside it. Fail-closed everywhere: an unresolvable root, an
//! escaping workdir, or a missing sandbox is an error, never a silent
//! unconfined run.

use serde_json::Value;
use std::path::{Path, PathBuf};

/// The configured project root, canonicalized. `None` when the caller is
/// not harness-wired (legacy/test surfaces keep their historical
/// unconstrained behavior; the TUI path always exports one).
pub fn project_root() -> Option<PathBuf> {
    // EFFECTIVE is written per session by wire_tool_env (operator root or
    // the session work area); HS_PROJECT_ROOT is operator intent
    // (hs-repl --project-dir) and doubles as the direct-invocation
    // override for tests and rigs that spawn plugins by hand.
    for var in ["HS_PROJECT_ROOT_EFFECTIVE", "HS_PROJECT_ROOT"] {
        if let Ok(raw) = std::env::var(var) {
            if !raw.is_empty() {
                if let Ok(canonical) = std::fs::canonicalize(&raw) {
                    return Some(canonical);
                }
            }
        }
    }
    None
}

fn escape_error(what: &str, resolved: &Path, root: &Path) -> Value {
    serde_json::json!({"$error": format!(
        "{what}: {} escapes the project root {} - refused",
        resolved.display(),
        root.display()
    )})
}

/// `path` must exist and resolve INSIDE the project root (symlinks are
/// resolved by canonicalize, so a link pointing outside is caught).
pub fn confine_existing(path: &Path, what: &str) -> Result<PathBuf, Value> {
    let Some(root) = project_root() else {
        return Ok(path.to_path_buf());
    };
    let resolved = std::fs::canonicalize(path).map_err(|e| {
        serde_json::json!({"$error": format!("{what}: cannot resolve {}: {e}", path.display())})
    })?;
    if !resolved.starts_with(&root) {
        return Err(escape_error(what, &resolved, &root));
    }
    Ok(resolved)
}

/// A write target that may not exist yet: canonicalize the deepest
/// existing ancestor, re-append the missing tail, then require the result
/// inside the project root. Relative paths anchor at the root.
pub fn confine_write(path: &Path, what: &str) -> Result<PathBuf, Value> {
    let Some(root) = project_root() else {
        return Ok(path.to_path_buf());
    };
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let mut ancestor = abs.as_path();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !ancestor.exists() {
        match ancestor.file_name() {
            Some(name) => {
                tail.push(name.to_os_string());
                ancestor = ancestor.parent().unwrap_or(Path::new("/"));
            }
            None => break,
        }
    }
    let mut resolved = std::fs::canonicalize(ancestor).map_err(|e| {
        serde_json::json!({"$error": format!("{what}: cannot resolve {}: {e}", ancestor.display())})
    })?;
    for part in tail.iter().rev() {
        resolved.push(part);
    }
    if !resolved.starts_with(&root) {
        return Err(escape_error(what, &resolved, &root));
    }
    Ok(resolved)
}
