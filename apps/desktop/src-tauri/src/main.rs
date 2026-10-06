//! HAIRSPRING desktop. The backend links the engine in-process (hs_loop::engine):
//! missions run inside this app, UI events stream to the webview as they happen.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use hs_loop::engine::Engine;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, State};

struct App {
    engine: Mutex<Option<Engine>>,
    busy: AtomicBool,
    queue: Mutex<std::collections::VecDeque<String>>,
    log_root: PathBuf,
    config: PathBuf,
}

fn home() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("hairspring")
}

/// Readiness of the default provider plus where state lives (first-run screen).
#[tauri::command]
fn setup_status(app: State<Arc<App>>) -> serde_json::Value {
    let config_exists = app.config.exists();
    let keys: Vec<String> = std::fs::read_dir(home().join("keys"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .collect();
    serde_json::json!({"config": app.config.display().to_string(), "config_exists": config_exists,
        "keys": keys, "ready": config_exists && !keys.is_empty()})
}

/// Save a provider key owner-only under ~/.config/hairspring/keys/<provider>.
#[tauri::command]
fn save_key(provider: String, key: String) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    if provider.is_empty() || !provider.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("bad provider name".into());
    }
    let dir = home().join("keys");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let f = dir.join(&provider);
    std::fs::write(&f, key.trim()).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_engine(app: State<Arc<App>>) -> Result<(), String> {
    let mut g = app.engine.lock().map_err(|e| e.to_string())?;
    if g.is_none() {
        std::fs::create_dir_all(&app.log_root).map_err(|e| e.to_string())?;
        *g = Some(Engine::open(&app.config, &app.log_root, hs_loop::appback::settings_get(&home())["max_steps"].as_u64().map(|n| n as u32)).map_err(|e| format!("{e:?}"))?);
    }
    Ok(())
}

fn workdir() -> PathBuf {
    std::env::var("HS_TERM_WORKDIR").map(PathBuf::from).unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// Run one goal on the engine, streaming events, then drain anything queued meanwhile.
fn run_and_drain(app: &App, window: &tauri::Window, goal: String) {
    let mut next = Some(goal);
    while let Some(g) = next.take() {
        let r = {
            let mut guard = match app.engine.lock() { Ok(g) => g, Err(_) => break };
            let Some(eng) = guard.as_mut() else { break };
            let w = window.clone();
            eng.run_goal(&g, move |ev| { let _ = w.emit("hs-event", ev); })
        };
        let done = match r {
            Ok(v) => v,
            Err(e) => serde_json::json!({"passed": false, "outcome": format!("{e:?}")}),
        };
        let _ = window.emit("hs-done", done);
        next = app.queue.lock().ok().and_then(|mut q| q.pop_front());
        let _ = window.emit("hs-queue", app.queue.lock().map(|q| q.iter().cloned().collect::<Vec<_>>()).unwrap_or_default());
    }
    app.busy.store(false, Ordering::SeqCst);
}

/// Submit a goal: runs now on a worker thread, or queues if a mission is already running.
#[tauri::command]
fn submit(app: State<'_, Arc<App>>, window: tauri::Window, goal: String) -> serde_json::Value {
    if app.busy.swap(true, Ordering::SeqCst) {
        let q = { let mut q = app.queue.lock().unwrap(); q.push_back(goal); q.iter().cloned().collect::<Vec<_>>() };
        let _ = window.emit("hs-queue", q.clone());
        return serde_json::json!({"queued": true, "queue": q});
    }
    let a = Arc::clone(&app);
    std::thread::spawn(move || run_and_drain(&a, &window, goal));
    serde_json::json!({"queued": false})
}

#[tauri::command]
fn slash_menu(prefix: String) -> serde_json::Value {
    serde_json::Value::Array(hs_loop::appback::slash_filter(&prefix))
}

/// Run a slash command. While a mission runs only queueing commands work.
#[tauri::command]
fn slash(app: State<'_, Arc<App>>, window: tauri::Window, line: String) -> serde_json::Value {
    let t = line.trim();
    if app.busy.load(Ordering::SeqCst) {
        for p in ["/goal ", "/plan "] {
            if let Some(rest) = t.strip_prefix(p) {
                let text = if p == "/plan " { format!("Use the plan tool first, then carry out the plan: {rest}") } else { rest.to_string() };
                let q = { let mut q = app.queue.lock().unwrap(); q.push_back(text); q.iter().cloned().collect::<Vec<_>>() };
                let _ = window.emit("hs-queue", q.clone());
                return serde_json::json!({"ok": true, "queued": rest});
            }
        }
        if t == "/queue" {
            return serde_json::json!({"ok": true, "queued": app.queue.lock().map(|q| q.iter().cloned().collect::<Vec<_>>()).unwrap_or_default()});
        }
        return serde_json::json!({"ok": false, "error": "a mission is running; only /goal, /plan and /queue work now"});
    }
    let mut g = match app.engine.lock() { Ok(g) => g, Err(e) => return serde_json::json!({"ok": false, "error": e.to_string()}) };
    let Some(eng) = g.as_mut() else { return serde_json::json!({"ok": false, "error": "engine not open"}) };
    let r = hs_loop::appback::run_slash(eng, &home(), t);
    drop(g);
    // /goal and /plan queue on the engine when idle: start them now.
    if r["queued"].is_string() && r["ok"] == true {
        let goal = app.engine.lock().ok().and_then(|mut g| g.as_mut().and_then(Engine::take_queued));
        if let Some(goal) = goal {
            return submit(app, window, goal);
        }
    }
    r
}

#[tauri::command]
fn questions() -> serde_json::Value {
    serde_json::Value::Array(hs_loop::appback::pending_questions(&home().join("ask")))
}

#[tauri::command]
fn answer(n: u32, text: String) -> Result<(), String> {
    hs_loop::appback::answer_question(&home().join("ask"), n, &text)
}

#[tauri::command]
fn trajectory(app: State<Arc<App>>, id: String) -> Result<serde_json::Value, String> {
    let g = app.engine.lock().map_err(|e| e.to_string())?;
    g.as_ref().ok_or("engine not open")?.trajectory(&id)
}

#[tauri::command]
fn plugins(app: State<Arc<App>>) -> Result<serde_json::Value, String> {
    hs_loop::appback::plugins(&app.config)
}

#[tauri::command]
fn settings() -> serde_json::Value {
    hs_loop::appback::settings_get(&home())
}

#[tauri::command]
fn save_settings(patch: serde_json::Value) -> Result<serde_json::Value, String> {
    hs_loop::appback::settings_set(&home(), &patch)?;
    Ok(hs_loop::appback::settings_get(&home()))
}

#[tauri::command]
fn export_session(app: State<Arc<App>>, id: String, path: String) -> Result<usize, String> {
    let g = app.engine.lock().map_err(|e| e.to_string())?;
    g.as_ref().ok_or("engine not open")?.export_session(&id, std::path::Path::new(&path))
}

#[tauri::command]
fn sessions(app: State<Arc<App>>) -> serde_json::Value {
    app.engine.lock().ok().and_then(|g| g.as_ref().map(Engine::sessions)).unwrap_or_else(|| serde_json::json!({"groups": []}))
}

#[tauri::command]
fn vitals(app: State<Arc<App>>) -> serde_json::Value {
    app.engine.lock().ok().and_then(|g| g.as_ref().map(Engine::vitals)).unwrap_or(serde_json::Value::Null)
}

#[tauri::command]
fn compact(app: State<Arc<App>>) {
    if let Ok(mut g) = app.engine.lock() {
        if let Some(e) = g.as_mut() {
            e.compact();
        }
    }
}

fn main() {
    let h = home();
    std::env::set_var("HS_ASK_DIR", h.join("ask"));
    std::env::set_var("HS_PERMISSION_FILE", h.join("permission"));
    let app = Arc::new(App { engine: Mutex::new(None), busy: AtomicBool::new(false), queue: Mutex::new(Default::default()), log_root: h.join("desktop-sessions"), config: h.join("hairspring.toml") });
    let ticker = Arc::clone(&app);
    tauri::Builder::default()
        .manage(app)
        .setup(move |tapp| {
            // schedule firing loop: every 30s, if idle, run due scheduled prompts
            let handle = tapp.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(30));
                if ticker.busy.load(Ordering::SeqCst) { continue; }
                let Ok(mut g) = ticker.engine.try_lock() else { continue };
                let Some(eng) = g.as_mut() else { continue };
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                let fired = eng.fire_due_schedules(&workdir(), now);
                if !fired.is_empty() { let _ = handle.emit("hs-scheduled", fired); }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![setup_status, save_key, open_engine, submit, sessions, vitals, compact, slash_menu, slash, questions, answer, trajectory, plugins, settings, save_settings, export_session])
        .run(tauri::generate_context!())
        .expect("error while running HAIRSPRING desktop");
}
