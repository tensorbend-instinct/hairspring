//! dsh-parity tools, workdir-scoped state under `<workdir>/.hs/`:
//! todo (todo_write), present, read_image, schedule (create/list/update/delete/due).
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn hs_dir(wd: &Path) -> PathBuf {
    wd.join(".hs")
}

fn rw_json(path: &Path) -> Value {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null)
}

fn write_json(wd: &Path, name: &str, v: &Value) -> bool {
    std::fs::create_dir_all(hs_dir(wd)).is_ok() && std::fs::write(hs_dir(wd).join(name), v.to_string()).is_ok()
}

/// Resolve a user path inside the workdir; refuse anything that escapes it.
fn inside(wd: &Path, p: &str) -> Result<PathBuf, String> {
    let root = wd.canonicalize().map_err(|e| e.to_string())?;
    let joined = if Path::new(p).is_absolute() { PathBuf::from(p) } else { root.join(p) };
    let c = joined.canonicalize().map_err(|_| format!("no such file: {p}"))?;
    if c.starts_with(&root) { Ok(c) } else { Err(format!("path escapes the workspace: {p}")) }
}

#[must_use]
pub fn todo(wd: &Path, args: &Value) -> Value {
    let file = hs_dir(wd).join("todo.json");
    let Some(items) = args["todos"].as_array() else {
        let cur = rw_json(&file);
        let list = if cur.is_array() { cur } else { json!([]) };
        return json!({"ok": true, "todos": list, "counts": counts(&list)});
    };
    for t in items {
        if t["content"].as_str().is_none_or(|s| s.trim().is_empty()) {
            return json!({"$error": "each todo needs content"});
        }
        if !["pending", "in_progress", "completed"].contains(&t["status"].as_str().unwrap_or("")) {
            return json!({"$error": "status must be pending, in_progress or completed"});
        }
    }
    if items.iter().filter(|t| t["status"] == "in_progress").count() > 1 {
        return json!({"$error": "only one todo may be in_progress"});
    }
    let list = Value::Array(items.clone());
    if !write_json(wd, "todo.json", &list) {
        return json!({"$error": "cannot save todos"});
    }
    json!({"ok": true, "todos": list, "counts": counts(&list)})
}

fn counts(list: &Value) -> Value {
    let n = |s: &str| list.as_array().map_or(0, |a| a.iter().filter(|t| t["status"] == s).count());
    json!({"pending": n("pending"), "in_progress": n("in_progress"), "completed": n("completed")})
}

fn mime_of(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "md" => "text/markdown",
        "txt" | "log" => "text/plain",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "csv" => "text/csv",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// Publish a workspace file to the user: recorded in `.hs/presented.jsonl` for the UI.
#[must_use]
pub fn present(wd: &Path, args: &Value) -> Value {
    let Some(p) = args["path"].as_str().filter(|s| !s.is_empty()) else {
        return json!({"$error": "path is required"});
    };
    let full = match inside(wd, p) {
        Ok(f) => f,
        Err(e) => return json!({"$error": e}),
    };
    let Ok(md) = std::fs::metadata(&full) else { return json!({"$error": "cannot stat file"}) };
    if !md.is_file() {
        return json!({"$error": "not a file"});
    }
    let rec = json!({"path": full.display().to_string(), "title": args["title"], "mime": mime_of(&full), "bytes": md.len()});
    let _ = std::fs::create_dir_all(hs_dir(wd));
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(hs_dir(wd).join("presented.jsonl")) {
        use std::io::Write;
        let _ = writeln!(f, "{rec}");
    }
    json!({"ok": true, "presented": rec})
}

fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        o.push(T[(n >> 18) as usize & 63] as char);
        o.push(T[(n >> 12) as usize & 63] as char);
        o.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        o.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    o
}

fn dims(b: &[u8]) -> Option<(&'static str, u32, u32)> {
    if b.len() > 24 && b.starts_with(&[137, 80, 78, 71]) {
        let w = u32::from_be_bytes(b[16..20].try_into().ok()?);
        let h = u32::from_be_bytes(b[20..24].try_into().ok()?);
        return Some(("image/png", w, h));
    }
    if b.len() > 10 && (b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a")) {
        return Some(("image/gif", u32::from(u16::from_le_bytes([b[6], b[7]])), u32::from(u16::from_le_bytes([b[8], b[9]]))));
    }
    if b.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2;
        while i + 9 < b.len() {
            if b[i] != 0xFF {
                i += 1;
                continue;
            }
            let m = b[i + 1];
            if (0xC0..=0xCF).contains(&m) && m != 0xC4 && m != 0xC8 && m != 0xCC {
                return Some(("image/jpeg", u32::from(u16::from_be_bytes([b[i + 7], b[i + 8]])), u32::from(u16::from_be_bytes([b[i + 5], b[i + 6]]))));
            }
            i += 2 + usize::from(u16::from_be_bytes([b[i + 2], b[i + 3]]));
        }
    }
    None
}

#[must_use]
pub fn read_image(wd: &Path, args: &Value) -> Value {
    let Some(p) = args["path"].as_str().filter(|s| !s.is_empty()) else {
        return json!({"$error": "path is required"});
    };
    let full = match inside(wd, p) {
        Ok(f) => f,
        Err(e) => return json!({"$error": e}),
    };
    let Ok(bytes) = std::fs::read(&full) else { return json!({"$error": "cannot read file"}) };
    let Some((mime, w, h)) = dims(&bytes) else { return json!({"$error": "not a PNG, JPEG or GIF image"}) };
    if bytes.len() > 5_000_000 {
        return json!({"$error": "image over 5 MB"});
    }
    json!({"ok": true, "mime": mime, "width": w, "height": h, "bytes": bytes.len(), "data_base64": b64(&bytes)})
}

/// Recurring-prompt store. `due` returns entries whose next_at <= now and advances them.
#[must_use]
pub fn schedule(wd: &Path, args: &Value) -> Value {
    let now = args["now"].as_u64().unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()));
    let file = hs_dir(wd).join("schedules.json");
    let mut list: Vec<Value> = rw_json(&file).as_array().cloned().unwrap_or_default();
    let save = |l: &Vec<Value>| write_json(wd, "schedules.json", &Value::Array(l.clone()));
    let min_ok = |s: u64| s >= 60;
    match args["op"].as_str().unwrap_or("list") {
        "create" => {
            let Some(prompt) = args["prompt"].as_str().filter(|s| !s.trim().is_empty()) else {
                return json!({"$error": "create needs prompt"});
            };
            let Some(every) = args["every_secs"].as_u64().filter(|s| min_ok(*s)) else {
                return json!({"$error": "every_secs must be at least 60"});
            };
            let id = format!("sch-{}-{}", now, list.len() + 1);
            let s = json!({"id": id, "prompt": prompt, "every_secs": every, "next_at": now + every});
            list.push(s.clone());
            if !save(&list) {
                return json!({"$error": "cannot save"});
            }
            json!({"ok": true, "schedule": s})
        }
        "list" => json!({"ok": true, "schedules": list}),
        "update" => {
            let id = args["id"].as_str().unwrap_or("");
            let Some(s) = list.iter_mut().find(|s| s["id"] == id) else { return json!({"$error": "no such schedule"}) };
            if let Some(p) = args["prompt"].as_str() {
                s["prompt"] = json!(p);
            }
            if let Some(e) = args["every_secs"].as_u64() {
                if !min_ok(e) {
                    return json!({"$error": "every_secs must be at least 60"});
                }
                s["every_secs"] = json!(e);
                s["next_at"] = json!(now + e);
            }
            let out = s.clone();
            if !save(&list) {
                return json!({"$error": "cannot save"});
            }
            json!({"ok": true, "schedule": out})
        }
        "delete" => {
            let id = args["id"].as_str().unwrap_or("");
            let before = list.len();
            list.retain(|s| s["id"] != id);
            let removed = list.len() != before;
            if removed && !save(&list) {
                return json!({"$error": "cannot save"});
            }
            json!({"ok": true, "deleted": removed})
        }
        "due" => {
            let mut due = vec![];
            for s in &mut list {
                if s["next_at"].as_u64().unwrap_or(u64::MAX) <= now {
                    due.push(s.clone());
                    let e = s["every_secs"].as_u64().unwrap_or(60);
                    s["next_at"] = json!(now + e);
                }
            }
            if !due.is_empty() && !save(&list) {
                return json!({"$error": "cannot save"});
            }
            json!({"ok": true, "due": due})
        }
        other => json!({"$error": format!("unknown op '{other}'")}),
    }
}

/// Named multi-step workflows: an ordered list of goal prompts the harness runs
/// back to back (`Engine::run_workflow`). ops: define {name, steps[]}, list, get, delete.
#[must_use]
pub fn workflow(wd: &Path, args: &Value) -> Value {
    let file = hs_dir(wd).join("workflows.json");
    let mut m = rw_json(&file).as_object().cloned().unwrap_or_default();
    let save = |m: &serde_json::Map<String, Value>| write_json(wd, "workflows.json", &Value::Object(m.clone()));
    let name = args["name"].as_str().unwrap_or("");
    match args["op"].as_str().unwrap_or("list") {
        "define" => {
            let steps: Vec<Value> = args["steps"].as_array().cloned().unwrap_or_default().into_iter()
                .filter(|s| s.as_str().is_some_and(|t| !t.trim().is_empty())).collect();
            if name.trim().is_empty() || steps.is_empty() {
                return json!({"$error": "define needs name and at least one non-empty step"});
            }
            m.insert(name.to_string(), Value::Array(steps));
            if !save(&m) {
                return json!({"$error": "cannot save"});
            }
            json!({"ok": true, "name": name, "steps": m[name]})
        }
        "get" => m.get(name).map_or_else(|| json!({"$error": "no such workflow"}), |v| json!({"ok": true, "name": name, "steps": v})),
        "list" => json!({"ok": true, "workflows": m.keys().collect::<Vec<_>>()}),
        "delete" => {
            let removed = m.remove(name).is_some();
            if removed && !save(&m) {
                return json!({"$error": "cannot save"});
            }
            json!({"ok": true, "deleted": removed})
        }
        other => json!({"$error": format!("unknown op '{other}'")}),
    }
}

/// Voice: speak text to a WAV file with a stock TTS command. `HS_TTS_CMD` is the
/// executable (default `piper`); `HS_TTS_MODEL` its voice model. The command gets
/// `--model <m> --output_file <out>` and the text on stdin. Output stays inside `wd`.
#[must_use]
pub fn speak(wd: &Path, args: &Value) -> Value {
    let Some(text) = args["text"].as_str().filter(|t| !t.trim().is_empty()) else {
        return json!({"$error": "speak needs text"});
    };
    if text.chars().count() > 2000 {
        return json!({"$error": "text over 2000 characters"});
    }
    let rel = args["path"].as_str().map_or_else(|| format!(".hs/voice/{}.wav", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis())), str::to_string);
    let rp = Path::new(&rel);
    if rp.is_absolute() || rp.components().any(|c| matches!(c, std::path::Component::ParentDir)) || rp.extension().is_none_or(|e| e != "wav") {
        return json!({"$error": "path must be a relative .wav path inside the workspace"});
    }
    let out = wd.join(rp);
    if let Some(dir) = out.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return json!({"$error": "cannot create output dir"});
        }
    }
    let cmd = std::env::var("HS_TTS_CMD").unwrap_or_else(|_| "piper".into());
    let Ok(model) = std::env::var("HS_TTS_MODEL") else {
        return json!({"$error": "no voice configured: set HS_TTS_MODEL (and HS_TTS_CMD if not piper)"});
    };
    let child = std::process::Command::new(&cmd).args(["--model", &model, "--output_file"]).arg(&out)
        .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped()).spawn();
    let Ok(mut child) = child else {
        return json!({"$error": format!("cannot start TTS command '{cmd}'")});
    };
    if let Some(mut si) = child.stdin.take() {
        use std::io::Write;
        let _ = si.write_all(text.as_bytes());
    }
    let Ok(o) = child.wait_with_output() else {
        return json!({"$error": "TTS command failed to finish"});
    };
    let bytes = std::fs::metadata(&out).map_or(0, |m| m.len());
    if !o.status.success() || bytes < 44 {
        return json!({"$error": format!("TTS failed: {}", String::from_utf8_lossy(&o.stderr).chars().take(200).collect::<String>())});
    }
    json!({"ok": true, "path": rel, "bytes": bytes})
}

/// File search by glob (dsh glob): gitignore-aware, sorted, capped.
#[must_use]
pub fn glob(wd: &Path, args: &Value) -> Value {
    let Some(pat) = args["pattern"].as_str().filter(|s| !s.is_empty()) else {
        return json!({"$error": "pattern is required"});
    };
    let base = match args["path"].as_str().filter(|s| !s.is_empty()) {
        Some(p) => match inside(wd, p) {
            Ok(b) => b,
            Err(e) => return json!({"$error": e}),
        },
        None => match wd.canonicalize() {
            Ok(b) => b,
            Err(e) => return json!({"$error": e.to_string()}),
        },
    };
    let limit = args["limit"].as_u64().unwrap_or(200).clamp(1, 1000) as usize;
    let mut ob = ignore::overrides::OverrideBuilder::new(&base);
    if let Err(e) = ob.add(pat) {
        return json!({"$error": format!("bad pattern: {e}")});
    }
    let Ok(ov) = ob.build() else { return json!({"$error": "bad pattern"}) };
    let mut files: Vec<String> = ignore::WalkBuilder::new(&base)
        .overrides(ov)
        .require_git(false)
        .build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter_map(|e| e.path().strip_prefix(&base).ok().map(|p| p.display().to_string()))
        .collect();
    files.sort();
    let truncated = files.len() > limit;
    files.truncate(limit);
    json!({"ok": true, "files": files, "truncated": truncated})
}
