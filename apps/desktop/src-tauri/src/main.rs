//! HAIRSPRING desktop shell. The UI is a thin client over the `hairspring`
//! binary: setup check, one-shot missions, streamed output. No logic is
//! duplicated here; the loop, checker and critic stay in the CLI.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use tauri::{Emitter, Manager};

fn hairspring_bin() -> String {
    std::env::var("HAIRSPRING_BIN").unwrap_or_else(|_| "hairspring".into())
}

/// First-run readiness: `hairspring setup --check` (exit 0 = a provider is ready).
#[tauri::command]
fn setup_check() -> Result<String, String> {
    let out = Command::new(hairspring_bin())
        .args(["setup", "--check"])
        .output()
        .map_err(|e| format!("cannot run {}: {e}", hairspring_bin()))?;
    let text = String::from_utf8_lossy(&[out.stdout, out.stderr].concat()).to_string();
    if out.status.success() { Ok(text) } else { Err(text) }
}

/// Run a one-shot mission, streaming each output line as a `mission-line` event.
#[tauri::command]
fn run_goal(app: tauri::AppHandle, goal: String, config: String, dir: String, project: String) -> Result<(), String> {
    let mut child = Command::new(hairspring_bin())
        .args(["run", "--goal", &goal, "--config", &config, "--dir", &dir, "--project-dir", &project])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let out = child.stdout.take().ok_or("no stdout")?;
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            let _ = app.emit("mission-line", line);
        }
        let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
        let _ = app.emit("mission-done", code);
    });
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![setup_check, run_goal])
        .setup(|app| {
            let _ = app.get_webview_window("main");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running HAIRSPRING desktop");
}
