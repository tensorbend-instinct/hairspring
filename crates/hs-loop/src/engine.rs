//! In-process engine for the desktop app: the same ReplSession the TUI drives,
//! exposed as plain JSON so a native UI links the crate instead of shelling
//! out to the CLI. Events stream through the caller's callback while a mission
//! runs; sessions, vitals and results are JSON values.
use crate::repl::{group_sessions_by_workspace, list_sessions, ReplSession};
use crate::LoopError;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub struct Engine {
    session: ReplSession,
    log_root: PathBuf,
    queue: std::collections::VecDeque<String>,
}

impl Engine {
    /// Open (or create) a session under `log_root` with the rig in `config`.
    /// Lenient: an uncredentialed default model still opens, so the app can
    /// show its setup screen instead of failing at launch.
    pub fn open(config: &Path, log_root: &Path, max_steps: Option<u32>) -> Result<Self, LoopError> {
        let session = ReplSession::load_lenient(config, log_root, true, max_steps)?;
        Ok(Self { session, log_root: log_root.to_path_buf(), queue: std::collections::VecDeque::new() })
    }

    /// Reopen an existing session (same stream: its history and missions
    /// carry on). The caller points HS_PROJECT_ROOT at the session's own
    /// workspace first (`appback::session_workspace`).
    pub fn open_resume(config: &Path, log_root: &Path, max_steps: Option<u32>, id: &str) -> Result<Self, LoopError> {
        let uid = uuid::Uuid::parse_str(id).map_err(|e| LoopError::Visibility(e.to_string()))?;
        let session = ReplSession::load_resume(config, log_root, true, max_steps, uid)?;
        Ok(Self { session, log_root: log_root.to_path_buf(), queue: std::collections::VecDeque::new() })
    }

    /// Run one goal to completion. `emit` receives every UI event as JSON
    /// (`{"type": "step"|"tool_start"|"tool_end"|"reasoning"|...}`) while the
    /// mission runs; the return value is the mission result.
    pub fn run_goal(
        &mut self,
        goal: &str,
        emit: impl FnMut(Value) + Send + 'static,
    ) -> Result<Value, LoopError> {
        self.run_sourced(goal,emit,true)
    }
    fn run_sourced(&mut self,goal:&str,emit:impl FnMut(Value)+Send+'static,human:bool)->Result<Value,LoopError>{
        let emit = Arc::new(Mutex::new(emit));
        let e2 = Arc::clone(&emit);
        self.session.set_ui_sink(Box::new(move |ev| {
            if let Ok(mut f) = e2.lock() {
                f(ev.to_json());
            }
        }));
        let r = if human{self.session.run_goal(goal)}else{self.session.run_external_goal(goal)}?;
        Ok(json!({
            "passed": r.passed, "outcome": r.outcome, "steps": r.steps,
            "model_calls": r.model_calls, "cost_micros": r.cost_micros,
            "budget_killed": r.budget_killed, "harness_error": r.harness_error,
            "stream_id": r.stream_id.to_string(),
        }))
    }

    pub fn run_goal_round(&mut self,emit:impl FnMut(Value)+Send+'static)->Result<Option<Value>,LoopError>{
        self.session.set_ui_sink(Box::new({let mut emit=emit;move|ev|emit(ev.to_json())}));
        self.session.run_next_goal_round().map(|r|r.map(|r|json!({"passed":r.passed,"outcome":r.outcome,"steps":r.steps,"model_calls":r.model_calls,"cost_micros":r.cost_micros,"budget_killed":r.budget_killed,"harness_error":r.harness_error,"stream_id":r.stream_id.to_string()})))
    }
    /// Sessions grouped by workspace, newest first (the sidebar).
    #[must_use]
    pub fn sessions(&self) -> Value {
        let infos = list_sessions(&self.log_root);
        let groups: Vec<Value> = group_sessions_by_workspace(&infos)
            .into_iter()
            .map(|(ws, v)| {
                json!({"workspace": ws, "sessions": v.iter().map(|i| json!({
                    "id": i.id.to_string(), "events": i.events, "title": i.preview.split(" ENVIRONMENT").next().unwrap_or("").trim(),
                    "age_secs": i.modified.elapsed().map_or(0, |d| d.as_secs())})).collect::<Vec<_>>()})
            })
            .collect();
        json!({"groups": groups})
    }

    #[must_use]
    pub fn vitals(&self) -> Value {
        let v = self.session.vitals();
        json!({"model": v.model_label, "missions": v.missions_run, "steps": v.total_steps,
            "model_calls": v.total_model_calls, "cost_micros": v.total_cost_micros,
            "elapsed_secs": v.elapsed.as_secs(), "stream_id": v.stream_id.to_string()})
    }

    /// Export one session's stream directory as a ZIP; returns the file count.
    pub fn export_session(&self, id: &str, out: &Path) -> Result<usize, String> {
        let uid = uuid::Uuid::parse_str(id).map_err(|e| e.to_string())?;
        let dir = self.log_root.join("streams").join(uid.to_string());
        crate::export::export_zip(&dir, out).map_err(|e| e.to_string())
    }

    /// The schedule firing loop body: run every schedule in `wd` that is due at
    /// `now` (unix secs) as a goal, in order. Returns one result per fired schedule:
    /// `{id, prompt, result|error}`. Call it on a timer (the app does, every 30s).
    pub fn fire_due_schedules(&mut self, wd: &Path, now: u64) -> Vec<Value> {
        let due = crate::tools2::schedule(wd, &json!({"op": "due", "now": now}));
        let mut out = vec![];
        for s in due["due"].as_array().cloned().unwrap_or_default() {
            let prompt = s["prompt"].as_str().unwrap_or("").to_string();
            let entry = match self.run_sourced(&prompt, |_| {},false) {
                Ok(r) => json!({"id": s["id"], "prompt": prompt, "result": r}),
                Err(e) => json!({"id": s["id"], "prompt": prompt, "error": e.to_string()}),
            };
            out.push(entry);
        }
        out
    }

    /// Run a defined workflow: each step as a goal in order; stops at the first
    /// step that does not pass. Returns `{ok, ran, results[]}`.
    pub fn run_workflow(&mut self, wd: &Path, name: &str) -> Value {
        let w = crate::tools2::workflow(wd, &json!({"op": "get", "name": name}));
        let Some(steps) = w["steps"].as_array().cloned() else {
            return json!({"ok": false, "error": "no such workflow", "ran": 0, "results": []});
        };
        let mut results = vec![];
        let mut ok = true;
        for st in steps {
            match self.run_sourced(st.as_str().unwrap_or(""), |_| {},false) {
                Ok(r) => {
                    let pass = r["passed"] == true;
                    results.push(r);
                    if !pass {
                        ok = false;
                        break;
                    }
                }
                Err(e) => {
                    results.push(json!({"error": e.to_string()}));
                    ok = false;
                    break;
                }
            }
        }
        json!({"ok": ok, "ran": results.len(), "results": results})
    }

    pub fn set_mode(&mut self, mode: &str) -> Result<(), String> {
        self.session.set_mode(mode)
    }

    #[must_use]
    pub fn tool_names(&self) -> Vec<String> {
        self.session.native_tool_names()
    }

    /// Queue-while-busy: goals wait here and run in order via `run_next`.
    pub fn queue_goal(&mut self, goal: &str) {
        self.queue.push_back(goal.to_string());
    }

    /// Take the newest queued goal (the app starts it immediately when idle).
    pub fn take_queued(&mut self) -> Option<String> {
        self.queue.pop_back()
    }

    #[must_use]
    pub fn queued(&self) -> Vec<String> {
        self.queue.iter().cloned().collect()
    }

    /// Run the oldest queued goal; `Ok(None)` when the queue is empty.
    pub fn run_next(&mut self, emit: impl FnMut(Value) + Send + 'static) -> Result<Option<Value>, LoopError> {
        let Some(g) = self.queue.pop_front() else { return Ok(None) };
        self.run_goal(&g, emit).map(Some)
    }

    pub fn set_model(&mut self, name: &str) -> Result<(), String> {
        self.session.set_model_override(Some(name.to_string())).map_err(|e| e.to_string())
    }

    /// Trajectory of one session: turns (model calls), tool calls, events, wall time.
    pub fn trajectory(&self, id: &str) -> Result<Value, String> {
        let uid = uuid::Uuid::parse_str(id).map_err(|e| e.to_string())?;
        let reader = hs_log::StreamReader::open(&self.log_root, uid).map_err(|e| e.to_string())?;
        let evs = reader.events().map_err(|e| e.to_string())?;
        let count = |k: hs_core::EventKind| evs.iter().filter(|e| e.kind == k).count() as u64;
        let (first, last) = (evs.first().map_or(0, |e| e.ts_wall_ms), evs.last().map_or(0, |e| e.ts_wall_ms));
        let model_ms: u64 = evs.iter().filter(|e| e.kind == hs_core::EventKind::ModelCall).map(|e| u64::from(e.latency_ms)).sum();
        let tool_ms: u64 = evs.iter().filter(|e| e.kind == hs_core::EventKind::ToolCall).map(|e| u64::from(e.latency_ms)).sum();
        Ok(json!({
            "turns": count(hs_core::EventKind::ModelCall), "calls": count(hs_core::EventKind::ToolCall),
            "events": evs.len() as u64, "duration_ms": u64::try_from((last - first).max(0)).unwrap_or(0),
            "model_ms": model_ms, "tool_ms": tool_ms,
            "cost_micros": evs.iter().map(|e| e.cost_usd_micros.max(0)).sum::<i64>(),
        }))
    }

    pub fn compact(&mut self) {
        self.session.request_compact();
    }

    #[must_use]
    pub fn models(&self) -> Value {
        json!(self.session.model_names().into_iter().map(|(n, d)| json!({"name": n, "default": d})).collect::<Vec<_>>())
    }
}
