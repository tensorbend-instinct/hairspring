//! Background jobs (dsh parity: job_list / job_output / job_kill + start).
//! State under `<workdir>/.hs/jobs/<id>.{json,out,exit}`; a job is alive while
//! its leader pid is in /proc and the exit file is absent.
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn dir(workdir: &Path) -> PathBuf {
    workdir.join(".hs").join("jobs")
}

fn alive(pid: u32, exit_file: &Path) -> bool {
    !exit_file.exists() && Path::new(&format!("/proc/{pid}")).exists()
}

fn meta(workdir: &Path, id: &str) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(dir(workdir).join(format!("{id}.json"))).ok()?).ok()
}

fn view(workdir: &Path, id: &str) -> Option<Value> {
    let m = meta(workdir, id)?;
    let pid = m["pid"].as_u64()? as u32;
    let exit_file = dir(workdir).join(format!("{id}.exit"));
    let running = alive(pid, &exit_file);
    let code = std::fs::read_to_string(&exit_file).ok().and_then(|s| s.trim().parse::<i64>().ok());
    Some(json!({"id": id, "command": m["command"], "pid": pid,
        "status": if running { "running" } else { "exited" }, "exit_code": code}))
}

#[must_use]
pub fn call(workdir: &Path, args: &Value) -> Value {
    let op = args["op"].as_str().unwrap_or("list");
    let d = dir(workdir);
    match op {
        "start" => {
            let Some(cmd) = args["command"].as_str() else {
                return json!({"$error": "start needs command"});
            };
            if std::fs::create_dir_all(&d).is_err() {
                return json!({"$error": "cannot create jobs dir"});
            }
            let n = std::fs::read_dir(&d).map_or(0, |r| r.filter(|e| e.as_ref().is_ok_and(|e| e.path().extension().is_some_and(|x| x == "json"))).count());
            let id = format!("j{}", n + 1);
            let out = d.join(format!("{id}.out"));
            let exit_file = d.join(format!("{id}.exit"));
            match crate::termexec::spawn_background(workdir, cmd, &out, &exit_file) {
                Ok(pid) => {
                    let _ = std::fs::write(d.join(format!("{id}.json")), json!({"command": cmd, "pid": pid}).to_string());
                    json!({"ok": true, "id": id, "pid": pid})
                }
                Err(e) => e,
            }
        }
        "list" => {
            let mut ids: Vec<String> = std::fs::read_dir(&d)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .filter_map(|e| e.file_name().to_str()?.strip_suffix(".json").map(String::from))
                .collect();
            ids.sort_by_key(|i| i.trim_start_matches('j').parse::<u32>().unwrap_or(0));
            json!({"ok": true, "jobs": ids.iter().filter_map(|i| view(workdir, i)).collect::<Vec<_>>()})
        }
        "output" => {
            let Some(id) = args["id"].as_str() else { return json!({"$error": "output needs id"}) };
            if view(workdir, id).is_none() {
                return json!({"$error": format!("no such job {id}")});
            }
            let text = std::fs::read_to_string(d.join(format!("{id}.out"))).unwrap_or_default();
            let tail = args["tail_bytes"].as_u64().map_or(20_000, |n| n as usize);
            let start = text.len().saturating_sub(tail);
            let start = (start..=text.len()).find(|i| text.is_char_boundary(*i)).unwrap_or(text.len());
            let mut v = view(workdir, id).unwrap_or(json!({}));
            v["output"] = json!(&text[start..]);
            v["truncated"] = json!(start > 0);
            v
        }
        "kill" => {
            let Some(id) = args["id"].as_str() else { return json!({"$error": "kill needs id"}) };
            let Some(m) = meta(workdir, id) else { return json!({"$error": format!("no such job {id}")}) };
            let pid = m["pid"].as_u64().unwrap_or(0);
            let ok = std::process::Command::new("kill").args(["-9", "--", &format!("-{pid}")]).status().is_ok_and(|s| s.success());
            let _ = std::fs::write(d.join(format!("{id}.exit")), "137");
            json!({"ok": true, "id": id, "signalled": ok})
        }
        other => json!({"$error": format!("unknown op '{other}'")}),
    }
}
