//! Backend for the desktop app's panels: question UI protocol, slash menu,
//! plugins list and settings. Pure functions over files, so the UI stays thin.
use crate::engine::Engine;
use serde_json::{json, Value};
use std::path::Path;

/// Unanswered questions published by the `ask_user_question` tool in `dir`.
#[must_use]
pub fn pending_questions(dir: &Path) -> Vec<Value> {
    let mut out = vec![];
    for n in 1..=999u32 {
        let q = dir.join(format!("q-{n}.json"));
        if !q.exists() {
            break;
        }
        if dir.join(format!("a-{n}.json")).exists() {
            continue;
        }
        if let Ok(v) = std::fs::read_to_string(&q).map(|s| serde_json::from_str::<Value>(&s).unwrap_or(Value::Null)) {
            out.push(json!({"n": n, "question": v["question"], "options": v["options"]}));
        }
    }
    out
}

/// Answer question `n`; the blocked tool call picks the file up.
pub fn answer_question(dir: &Path, n: u32, answer: &str) -> Result<(), String> {
    if !dir.join(format!("q-{n}.json")).exists() {
        return Err(format!("no question {n}"));
    }
    std::fs::write(dir.join(format!("a-{n}.json")), json!({"answer": answer}).to_string()).map_err(|e| e.to_string())
}

const SLASH: [(&str, &str); 10] = [
    ("/goal", "Run a goal now (queued if busy)"),
    ("/plan", "Plan first with the plan tool, then act"),
    ("/feedback", "Record feedback for this session"),
    ("/compact", "Compact older history"),
    ("/permission", "Set permission: ask or auto"),
    ("/model", "List models or switch: /model <name>"),
    ("/export", "Export a session as ZIP: /export <session-id> <path>"),
    ("/add-file", "Attach a file to the next goal: /add-file <path>"),
    ("/mode", "standard | ptc | minimal | creator"),
    ("/queue", "Show queued goals"),
];

#[must_use]
pub fn slash_commands() -> Vec<Value> {
    SLASH.iter().map(|(n, d)| json!({"name": n, "description": d})).collect()
}

/// Commands whose name starts with the typed prefix (the menu as you type).
#[must_use]
pub fn slash_filter(prefix: &str) -> Vec<Value> {
    slash_commands().into_iter().filter(|c| c["name"].as_str().is_some_and(|n| n.starts_with(prefix))).collect()
}

/// Run one slash command line. `wd` holds app state (settings, feedback).
pub fn run_slash(eng: &mut Engine, wd: &Path, line: &str) -> Value {
    let line = line.trim();
    let (cmd, rest) = line.split_once(' ').map_or((line, ""), |(c, r)| (c, r.trim()));
    let err = |m: &str| json!({"ok": false, "error": m});
    match cmd {
        "/goal" if !rest.is_empty() => {
            eng.queue_goal(rest);
            json!({"ok": true, "queued": rest})
        }
        "/plan" if !rest.is_empty() => {
            eng.queue_goal(&format!("Use the plan tool first, then carry out the plan: {rest}"));
            json!({"ok": true, "queued": rest, "plan": true})
        }
        "/feedback" if !rest.is_empty() => {
            let _ = std::fs::create_dir_all(wd);
            let mut f = match std::fs::OpenOptions::new().create(true).append(true).open(wd.join("feedback.jsonl")) {
                Ok(f) => f,
                Err(e) => return err(&e.to_string()),
            };
            use std::io::Write;
            let _ = writeln!(f, "{}", json!({"feedback": rest}));
            json!({"ok": true})
        }
        "/compact" => {
            eng.compact();
            json!({"ok": true})
        }
        "/permission" => match rest {
            "ask" | "auto" => match settings_set(wd, &json!({"permission": rest})) {
                Ok(()) => json!({"ok": true, "permission": rest}),
                Err(e) => err(&e),
            },
            _ => json!({"ok": true, "permission": settings_get(wd)["permission"]}),
        },
        "/model" => {
            if rest.is_empty() {
                json!({"ok": true, "models": eng.models()})
            } else {
                match eng.set_model(rest) {
                    Ok(()) => json!({"ok": true, "model": rest}),
                    Err(e) => err(&e),
                }
            }
        }
        "/export" => {
            let mut it = rest.split_whitespace();
            let (Some(id), Some(out)) = (it.next(), it.next()) else { return err("usage: /export <session-id> <path>") };
            match eng.export_session(id, Path::new(out)) {
                Ok(n) => json!({"ok": true, "files": n, "path": out}),
                Err(e) => err(&e),
            }
        }
        "/add-file" if !rest.is_empty() => match std::fs::read_to_string(rest) {
            Ok(t) => json!({"ok": true, "attached": format!("[file {rest}]\n{t}")}),
            Err(e) => err(&format!("cannot read {rest}: {e}")),
        },
        "/mode" => match eng.set_mode(rest) {
            Ok(()) => {
                let _ = settings_set(wd, &json!({"mode": rest}));
                json!({"ok": true, "mode": rest})
            }
            Err(e) => err(&e),
        },
        "/queue" => json!({"ok": true, "queued": eng.queued()}),
        _ => err(&format!("unknown or incomplete command: {line}")),
    }
}

/// Tools and models configured in the rig (the Plugins panel).
pub fn plugins(config: &Path) -> Result<Value, String> {
    let t: toml::Value = std::fs::read_to_string(config).map_err(|e| e.to_string())?.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let list = |key: &str| -> Vec<Value> {
        t.get(key).and_then(|v| v.as_array()).map(|a| {
            a.iter().map(|e| json!({
                "name": e.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                "command": e.get("command").and_then(|v| v.as_array()).and_then(|c| c.first()).and_then(|v| v.as_str()).unwrap_or(""),
                "default": e.get("default").and_then(toml::Value::as_bool).unwrap_or(false),
            })).collect()
        }).unwrap_or_default()
    };
    Ok(json!({"tools": list("tools"), "models": list("models")}))
}

const SETTING_KEYS: [&str; 5] = ["mode", "max_steps", "permission", "theme", "project_dir"];

#[must_use]
/// Is the project folder missions are confined to empty? An empty folder is
/// the common "model cannot find my file" cause, so the app warns up front.
/// Dot-entries (the harness's own .hs scaffolding) do not count.
#[must_use]
pub fn project_status(root: &Path) -> Value {
    let canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut entries: Vec<String> = std::fs::read_dir(&canon)
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| !n.starts_with('.')).collect())
        .unwrap_or_default();
    entries.sort();
    entries.truncate(40);
    json!({"root": canon.display().to_string(), "empty": entries.is_empty(), "entries": entries})
}

pub fn settings_get(dir: &Path) -> Value {
    let mut s = json!({"mode": "standard", "max_steps": 30, "permission": "auto", "theme": "dark"});
    if let Some(saved) = std::fs::read_to_string(dir.join("settings.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        for k in SETTING_KEYS {
            if !saved[k].is_null() {
                s[k] = saved[k].clone();
            }
        }
    }
    s
}

pub fn settings_set(dir: &Path, patch: &Value) -> Result<(), String> {
    let obj = patch.as_object().ok_or("settings patch must be an object")?;
    let mut s = settings_get(dir);
    for (k, v) in obj {
        if !SETTING_KEYS.contains(&k.as_str()) {
            return Err(format!("unknown setting '{k}'"));
        }
        match k.as_str() {
            "mode" if !v.as_str().is_some_and(|m| crate::modes::MODES.contains(&m)) => return Err("mode must be standard|ptc|minimal|creator".into()),
            "permission" if !matches!(v.as_str(), Some("ask" | "auto")) => return Err("permission must be ask|auto".into()),
            "max_steps" if !v.as_u64().is_some_and(|n| (1..=1000).contains(&n)) => return Err("max_steps must be 1..1000".into()),
            "project_dir" if !v.as_str().is_some_and(|p| std::path::Path::new(p).is_absolute() && std::path::Path::new(p).is_dir()) => return Err("project_dir must be an existing absolute folder".into()),
            _ => {}
        }
        s[k] = v.clone();
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    // the dispatcher reads this file (HS_PERMISSION_FILE) on every mutating call
    std::fs::write(dir.join("permission"), s["permission"].as_str().unwrap_or("auto")).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("settings.json"), s.to_string()).map_err(|e| e.to_string())
}

// ---- workspaces (dsh parity): registered real folders, browse dialog, switch ----

fn ws_file(dir: &Path) -> std::path::PathBuf { dir.join("workspaces.json") }

fn ws_read(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(ws_file(dir)).ok().and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok()).unwrap_or_default()
}

fn ws_write(dir: &Path, v: &[String]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(ws_file(dir), serde_json::to_string(v).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

fn ws_entry(p: &str) -> Value {
    json!({"path": p, "name": Path::new(p).file_name().map_or_else(|| p.to_string(), |n| n.to_string_lossy().to_string())})
}

/// Registered workspaces: real folders, in the order they were added.
#[must_use]
pub fn workspaces_list(dir: &Path) -> Value {
    Value::Array(ws_read(dir).iter().map(|p| ws_entry(p)).collect())
}

/// Register a real folder as a workspace (canonical path, deduped). A
/// relative or missing path is refused: a workspace is always a real folder.
pub fn workspace_add(dir: &Path, path: &str) -> Result<Value, String> {
    let p = Path::new(path);
    if !p.is_absolute() { return Err("workspace must be an absolute folder path".into()); }
    let canon = p.canonicalize().map_err(|_| format!("no such folder: {path}"))?;
    if !canon.is_dir() { return Err(format!("not a folder: {path}")); }
    let c = canon.display().to_string();
    let mut v = ws_read(dir);
    if !v.contains(&c) { v.push(c.clone()); ws_write(dir, &v)?; }
    Ok(ws_entry(&c))
}

pub fn workspace_remove(dir: &Path, path: &str) -> Result<(), String> {
    let mut v = ws_read(dir);
    v.retain(|p| p != path);
    ws_write(dir, &v)
}

/// Make a registered workspace the project folder: new missions run with it
/// as their root and cwd (the app reopens its engine on it).
pub fn workspace_switch(dir: &Path, path: &str) -> Result<(), String> {
    if !ws_read(dir).iter().any(|p| p == path) { return Err(format!("not a registered workspace: {path}")); }
    settings_set(dir, &json!({"project_dir": path}))
}

/// The in-app folder browser (dsh's "-browse" picker): visible subfolders of
/// `path`, sorted, plus the parent for going up.
pub fn browse_dir(path: &str) -> Result<Value, String> {
    let canon = Path::new(path).canonicalize().map_err(|_| format!("no such folder: {path}"))?;
    if !canon.is_dir() { return Err(format!("not a folder: {path}")); }
    let mut dirs: Vec<String> = std::fs::read_dir(&canon).map_err(|e| e.to_string())?
        .flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| !n.starts_with('.')).collect();
    dirs.sort();
    Ok(json!({"path": canon.display().to_string(), "parent": canon.parent().map(|p| p.display().to_string()), "dirs": dirs}))
}

/// The project folder a session ran in (recorded when its mission ran).
#[must_use]
pub fn session_workspace(log_root: &Path, id: &str) -> Option<String> {
    uuid::Uuid::parse_str(id).ok()?;
    std::fs::read_to_string(log_root.join("session_workspace").join(id)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}
