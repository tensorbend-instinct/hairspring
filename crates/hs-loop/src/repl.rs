//! REPL surface (Eric 2026-09-07): run the HAIRSPRING harness end-to-end on
//! a goal from the CLI - one-shot (`hs-repl run --goal ...`) or interactive
//! (`hs-repl`, one goal per line, `:`-prefixed commands).
//!
//! This module is the testable core; the bin is a thin stdin/stdout shell
//! over it. Everything here reuses the production machinery: `swe_kernel` +
//! `InnerLoop`, `require_visibility` gate included.

use crate::{require_visibility, swe_kernel, InnerLoop, LoopError, MissionResult};
use std::path::{Path, PathBuf};

/// One parsed input line of the interactive REPL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplCommand {
    /// A goal to run end-to-end (any non-`:` line of text).
    Goal(String),
    Help,
    /// Steps/cost/stream of the current session.
    Status,
    /// Recovery tier B: snapshot the mission workdir (books `snapshot_ref`).
    Snapshot,
    /// Recovery tier B: hash-verified restore from a snapshot id (books
    /// the recovery on the mission stream).
    Restore(String),
    /// Print the input history (preloaded + this session's lines).
    History,
    /// Print the latest mission's answer artifact.
    LastAnswer,
    Quit,
    /// `:`-prefixed input that names no command.
    Unknown(String),
}

#[must_use]
pub fn parse_command(line: &str) -> ReplCommand {
    let t = line.trim();
    match t {
        ":quit" | ":q" | ":exit" => ReplCommand::Quit,
        ":help" | ":h" | ":?" => ReplCommand::Help,
        ":status" => ReplCommand::Status,
        ":snapshot" => ReplCommand::Snapshot,
        t if t.starts_with(":restore") => ReplCommand::Restore(
            t.trim_start_matches(":restore").trim().to_string(),
        ),
        ":history" => ReplCommand::History,
        ":last" => ReplCommand::LastAnswer,
        _ if t.starts_with(':') => ReplCommand::Unknown(t.to_string()),
        _ => ReplCommand::Goal(t.to_string()),
    }
}

/// Path-safe mission id from goal text: lowercase, alnum runs joined by
/// single dashes, capped at 40 chars. The id names the work dir
/// (`log_root/work`/<id>/answer.txt), so it must never contain separators.
#[must_use]
pub fn goal_slug(goal: &str) -> String {
    let mut out = String::with_capacity(goal.len().min(41));
    let mut dash = false;
    for c in goal.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
        if out.len() >= 40 {
            break;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "goal".to_string()
    } else {
        out
    }
}

pub const REPL_HELP: &str = "hairspring REPL - run the harness on a goal, end to end
  <text>    run <text> as a goal (mission) on the loaded kernel
  :status   steps, model calls, cost, stream id of the current session
  :snapshot snapshot the mission workdir (recovery tier B, books snapshot_ref)
  :restore <id>  restore the workdir from a snapshot (tier B, hash-verified)
  :history  input history - preloaded from this dir's prior sessions
  :last     print the latest mission's answer artifact
  :help     this text
  :quit     exit";

/// UI gap #1: a typed snapshot of where the session stands - the data
/// an ambient status bar paints. No string-scraping of logs: the session
/// accumulates every mission's counters as they land.
/// D4: the budget every session arms when the config declares none -
/// $10, matching the standing external exec guard. Explicit
/// `--budget-micros` or the config stanza overrides it.
pub const DEFAULT_SESSION_BUDGET_MICROS: u64 = 10_000_000;

#[derive(Debug, Clone)]
pub struct SessionVitals {
    /// Name of the configured default model.
    pub model_label: String,
    /// Missions completed in this session.
    pub missions_run: u64,
    /// Model steps across all missions.
    pub total_steps: u64,
    /// Provider calls across all missions.
    pub total_model_calls: u64,
    /// Accumulated cost in USD micros (as metered by the loop).
    pub total_cost_micros: u64,
    /// D5: conservative list-rate counterpart - the guarded figure.
    pub conservative_cost_micros: u64,
    /// Wall time since the session loaded its kernel.
    pub elapsed: std::time::Duration,
    /// The live log stream.
    pub stream_id: uuid::Uuid,
}

/// A loaded kernel + loop pair that runs goals as missions, in order.
pub struct ReplSession {
    inner: InnerLoop,
    used_ids: std::collections::HashSet<String>,
    last_answer_path: Option<PathBuf>,
    mcp_catalog: String,
    model_label: String,
    started: std::time::Instant,
    missions_run: u64,
    total_steps: u64,
    total_model_calls: u64,
    ui_flush: Option<Box<dyn FnMut() + Send>>,
}

impl ReplSession {

/// Gap #6: the REPL's compaction budget comes from the configured
/// model's real window, not the loop's ~1M-token default. The default
/// model's `context_tokens` stanza declares the window; the loop
/// budget keeps 25% headroom for the reply. Without this, a long
/// REPL session blows the provider's context (400) long before the
/// assembler's compactor would ever fire.
/// UI gap #1: the configured default model's NAME, for the status bar.
/// Same TOML stanza walk as `configured_context_tokens`.
fn configured_model_label(config: &Path) -> Option<String> {
    let text = std::fs::read_to_string(config).ok()?;
    let v: toml::Value = toml::from_str(&text).ok()?;
    let models = v.get("models")?.as_array()?;
    let default_model = models
        .iter()
        .find(|m| {
            m.get("default")
                .and_then(toml::Value::as_bool)
                .unwrap_or(false)
        })
        .or(models.first())?;
    default_model.get("name")?.as_str().map(str::to_string)
}

/// D4: the run's USD budget from the config, when declared
/// (`[run] budget_usd = <dollars>` or `budget_micros = <micros>`;
/// dollars win when both appear). Missing means the caller arms the
/// default session cap - "uncapped" is never the silent default
/// (live burn 2026-09-09: 0 missions, 58 calls, $8.81, no cap armed).
fn configured_budget_micros(config: &Path) -> Option<u64> {
    let text = std::fs::read_to_string(config).ok()?;
    let v: toml::Value = toml::from_str(&text).ok()?;
    let run = v.get("run")?.as_table()?;
    if let Some(usd) = run.get("budget_usd").and_then(toml::Value::as_float) {
        return Some((usd.max(0.0) * 1e6).round() as u64);
    }
    run.get("budget_micros")
        .and_then(toml::Value::as_integer)
        .map(|n| n.max(0) as u64)
}

fn configured_context_tokens(config: &Path) -> Option<usize> {
    let text = std::fs::read_to_string(config).ok()?;
    let v: toml::Value = toml::from_str(&text).ok()?;
    let models = v.get("models")?.as_array()?;
    let default_model = models
        .iter()
        .find(|m| {
            m.get("default")
                .and_then(toml::Value::as_bool)
                .unwrap_or(false)
        })
        .or(models.first())?;
    default_model
        .get("context_tokens")?
        .as_integer()
        .map(|n| n as usize)
}

    /// Tool-env wiring (live defect 2026-09-08): hs-tb-run sets
    /// `HS_TERM_WORKDIR/HS_SWE_WORKSPACE` for every plugin the kernel
    /// spawns; hs-repl never did, so term.exec fell back to /app and
    /// died `spawn: No such file or directory` on any host without one.
    /// The REPL wires it itself: the session dir IS the working
    /// directory, stable across missions - the operator exports nothing.
    /// Plugins are long-lived processes spawned at kernel load, so this
    /// must run BEFORE `swe_kernel`.
    fn wire_tool_env(log_root: &Path) -> Result<(), LoopError> {
        std::fs::create_dir_all(log_root)?;
        // D6 (live burn 2026-09-09): anchor the repo tools at the
        // session's WORK area, never the run root - the root holds
        // harness state (streams/, blobs/, memory.db) the mission must
        // not read through repo.read/repo.search.
        let work = log_root.join("work");
        std::fs::create_dir_all(&work)?;
        unsafe {
            std::env::set_var("HS_TERM_WORKDIR", &work);
            std::env::set_var("HS_SWE_WORKSPACE", &work);
            // The REPL serves the tb surface (tb_tools: term.exec works
            // the LIVE machine, no edit.patch candidate), so the blind
            // checker must run its .hs/checks against the real workdir
            // too - candidate mode could never be satisfied here.
            std::env::set_var("HS_SELFCHECK_DIRECT", "1");
        }
        Ok(())
    }

    /// Load the kernel from `config`, gated by `require_visibility` (the
    /// production startup rule: no blind runs), and build the loop on
    /// `log_root`.
    pub fn load(
        config: &Path,
        log_root: &Path,
        feedback: bool,
        max_steps: u32,
    ) -> Result<Self, LoopError> {
        // MCP tool seam (Eric 2026-09-07 web-tooling order): HS_MCP_SERVERS
        // points at a [[mcp_servers]] TOML; discovered tools merge into the
        // session kernel (mcp.<server>.<tool>) and are advertised on every
        // goal prompt so a real model can find them (REPL missions deliver
        // no native schema today - the prompt IS the tool catalog).
        let mut mcp_catalog = String::new();
        // Gap #1 (parity build 2026-09-07): native tool schemas on every
        // operator call, same as hs-tb-run/hs-swe-run - SYSTEM_NATIVE +
        // tool_choice:"required" holds the model on-protocol by construction.
        // The free-form path provably cannot (live proof: 39 prose replies
        // in 40 steps under explicit nudges).
        // D1 (dance #94): the advertised surface is DERIVED from the kernel
        // registry below (schemas_for_registry), never seeded from a flavor
        // list - a hardcoded tb_tools seed advertised term.exec to a SWE
        // config that never registered it (live DeepSeek failure 2026-09-09).
        let mut mcp_native: Vec<serde_json::Value> = Vec::new();
        let merged_config;
        let config = if let Ok(servers_toml) = std::env::var("HS_MCP_SERVERS") {
            let (fragment, native) =
                crate::mcpbridge::discover_mcp_tools(std::path::Path::new(&servers_toml))
                    .map_err(LoopError::Visibility)?;
            mcp_native.extend(native.iter().cloned());
            for t in &native {
                mcp_catalog.push_str(&format!(
                    "- {}: {}\n",
                    t["function"]["name"].as_str().unwrap_or(""),
                    t["function"]["description"].as_str().unwrap_or("")
                ));
            }
            let mut text = std::fs::read_to_string(config)?;
            text.push_str(&fragment);
            merged_config = log_root.join("repl-hairspring.toml");
            std::fs::write(&merged_config, text)?;
            merged_config.as_path()
        } else {
            config
        };
        Self::wire_tool_env(log_root)?;
        let kernel = swe_kernel(config, log_root)?;
        require_visibility(&kernel).map_err(LoopError::Visibility)?;
        let registered: Vec<String> = kernel
            .list_tools("operator")
            .into_iter()
            .map(|e| e.name)
            .collect();
        let mut native_tools =
            crate::toolschema::schemas_for_registry(&registered, &mcp_native, "applypatch");
        // Eric's five #5: the interactive surface offers delegation.
        native_tools.push(crate::toolschema::agent_spawn_tool());
        native_tools.push(crate::toolschema::agent_spawn_poll_tool());
        let mut inner = InnerLoop::new(kernel, log_root, feedback, max_steps)?;
        if let Some(tokens) = Self::configured_context_tokens(config) {
            inner.set_context_budget_tokens(tokens * 3 / 4);
        }
        inner.set_budget_micros(
            Self::configured_budget_micros(config).unwrap_or(DEFAULT_SESSION_BUDGET_MICROS),
        );
        // B1 (v5 D3): every REPL session owns the shared K plane at
        // <dir>/memory.db; the model consults it via the memory.recall
        // tool (cut #10: consulted, never pre-passed) and every mission
        // close distills into it, so each session compounds. If the db
        // cannot be created the tool simply is not offered (fail-open).
        let memory_db = log_root.join("memory.db");
        if hs_memory::sqlite::SqliteMemoryStore::open(&memory_db).is_ok() {
            native_tools.push(crate::toolschema::memory_recall_tool());
            inner.set_memory_db(&memory_db);
        }
        // B2 (v5 gate 6): every REPL session also joins the shared world
        // plane; the model writes proposals through the world.* tools and
        // the world service alone writes consequences (spec 3.4).
        inner.attach_world();
        native_tools.extend([
            crate::toolschema::world_propose_tool(),
            crate::toolschema::world_observe_tool(),
            crate::toolschema::world_install_tool(),
            crate::toolschema::world_tick_tool(),
        ]);
        inner.set_tools(serde_json::Value::Array(native_tools));
        let model_label = Self::configured_model_label(config).unwrap_or_else(|| "?".to_string());
        Ok(ReplSession {
            inner,
            used_ids: std::collections::HashSet::new(),
            last_answer_path: None,
            mcp_catalog,
            model_label,
            started: std::time::Instant::now(),
            missions_run: 0,
            total_steps: 0,
            total_model_calls: 0,
            ui_flush: None,
        })
    }

    /// Gap #4 (fork): branch an existing stream into a new linked stream
    /// carrying the parent's full transcript (`hs_log::StreamWriter::fork`),
    /// then run missions on the branch. The parent stream is untouched.
    pub fn load_fork(
        config: &Path,
        log_root: &Path,
        feedback: bool,
        max_steps: u32,
        parent: uuid::Uuid,
    ) -> Result<Self, LoopError> {
        let (child, child_writer) = hs_log::StreamWriter::fork(log_root, parent)?;
        drop(child_writer);
        Self::load_resume(config, log_root, feedback, max_steps, child)
    }

    /// Gap #4 (resume): adopt an existing stream instead of opening a fresh
    /// one. The substrate (`StreamWriter::resume` via `InnerLoop::with_stream`)
    /// recovers sequence + hash-chain state, so new missions append to the
    /// same stream and their prompts replay the prior transcript from the
    /// log. Config/MCP/native-tool setup is identical to `load()`.
    pub fn load_resume(
        config: &Path,
        log_root: &Path,
        feedback: bool,
        max_steps: u32,
        stream_id: uuid::Uuid,
    ) -> Result<Self, LoopError> {
        let mut mcp_catalog = String::new();
        // D1 (dance #94): advertised surface derives from the registry
        // below, same as load() - never a flavor-seeded list.
        let mut mcp_native: Vec<serde_json::Value> = Vec::new();
        let merged_config;
        let config = if let Ok(servers_toml) = std::env::var("HS_MCP_SERVERS") {
            let (fragment, native) =
                crate::mcpbridge::discover_mcp_tools(std::path::Path::new(&servers_toml))
                    .map_err(LoopError::Visibility)?;
            mcp_native.extend(native.iter().cloned());
            for t in &native {
                mcp_catalog.push_str(&format!(
                    "- {}: {}\n",
                    t["function"]["name"].as_str().unwrap_or(""),
                    t["function"]["description"].as_str().unwrap_or("")
                ));
            }
            let mut text = std::fs::read_to_string(config)?;
            text.push_str(&fragment);
            merged_config = log_root.join("repl-hairspring.toml");
            std::fs::write(&merged_config, text)?;
            merged_config.as_path()
        } else {
            config
        };
        Self::wire_tool_env(log_root)?;
        let kernel = swe_kernel(config, log_root)?;
        require_visibility(&kernel).map_err(LoopError::Visibility)?;
        let registered: Vec<String> = kernel
            .list_tools("operator")
            .into_iter()
            .map(|e| e.name)
            .collect();
        let mut native_tools =
            crate::toolschema::schemas_for_registry(&registered, &mcp_native, "applypatch");
        // Eric's five #5: the interactive surface offers delegation.
        native_tools.push(crate::toolschema::agent_spawn_tool());
        native_tools.push(crate::toolschema::agent_spawn_poll_tool());
        let mut inner = InnerLoop::with_stream(kernel, log_root, stream_id, feedback, max_steps)?;
        if let Some(tokens) = Self::configured_context_tokens(config) {
            inner.set_context_budget_tokens(tokens * 3 / 4);
        }
        inner.set_budget_micros(
            Self::configured_budget_micros(config).unwrap_or(DEFAULT_SESSION_BUDGET_MICROS),
        );
        // B1 (v5 D3): every REPL session owns the shared K plane at
        // <dir>/memory.db; the model consults it via the memory.recall
        // tool (cut #10: consulted, never pre-passed) and every mission
        // close distills into it, so each session compounds. If the db
        // cannot be created the tool simply is not offered (fail-open).
        let memory_db = log_root.join("memory.db");
        if hs_memory::sqlite::SqliteMemoryStore::open(&memory_db).is_ok() {
            native_tools.push(crate::toolschema::memory_recall_tool());
            inner.set_memory_db(&memory_db);
        }
        // B2 (v5 gate 6): every REPL session also joins the shared world
        // plane; the model writes proposals through the world.* tools and
        // the world service alone writes consequences (spec 3.4).
        inner.attach_world();
        native_tools.extend([
            crate::toolschema::world_propose_tool(),
            crate::toolschema::world_observe_tool(),
            crate::toolschema::world_install_tool(),
            crate::toolschema::world_tick_tool(),
        ]);
        inner.set_tools(serde_json::Value::Array(native_tools));
        let model_label = Self::configured_model_label(config).unwrap_or_else(|| "?".to_string());
        Ok(ReplSession {
            inner,
            used_ids: std::collections::HashSet::new(),
            last_answer_path: None,
            mcp_catalog,
            model_label,
            started: std::time::Instant::now(),
            missions_run: 0,
            total_steps: 0,
            total_model_calls: 0,
            ui_flush: None,
        })
    }

    /// The mission id the NEXT run of this goal would use: the slug, with
    /// a -2/-3/... suffix when the slug already ran in this session.
    pub fn mission_id_for(&self, goal: &str) -> String {
        let base = goal_slug(goal);
        if !self.used_ids.contains(&base) {
            return base;
        }
        for n in 2.. {
            let cand = format!("{base}-{n}");
            if !self.used_ids.contains(&cand) {
                return cand;
            }
        }
        unreachable!("unbounded counter always finds a free id")
    }

    /// Run one goal end-to-end: mission id names the work dir, the goal
    /// text itself is the prompt the model sees.
    pub fn run_goal(&mut self, goal: &str) -> Result<MissionResult, LoopError> {
        // Dance #95: production missions always run with mission memory
        // armed - the feedback flag belongs to the experiment binaries
        // (baseline arm), never to an operator's TUI mission.
        self.inner.arm_mission_memory();
        let id = self.mission_id_for(goal);
        let prompt = if self.mcp_catalog.is_empty() {
            goal.to_string()
        } else {
            format!("{goal}\n\nAVAILABLE MCP TOOLS (call them like any other tool):\n{}", self.mcp_catalog)
        };
        let r = self.inner.run_mission_full(&id, &prompt)?;
        self.missions_run += 1;
        self.total_steps += u64::from(r.steps);
        self.total_model_calls += u64::from(r.model_calls);
        self.used_ids.insert(id);
        self.last_answer_path = Some(r.answer_path.clone());
        Ok(r)
    }

    /// The latest mission's answer artifact, read fresh from disk.
    pub fn last_answer(&self) -> Option<String> {
        self.last_answer_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
    }

    /// Names of the native tool schemas delivered on operator model calls
    /// (builtin + MCP-discovered). Empty only if delivery was never set up.
    pub fn native_tool_names(&self) -> Vec<String> {
        self.inner
            .native_tools()
            .and_then(|t| t.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|t| t["function"]["name"].as_str().map(str::to_string))
            .collect()
    }

    pub fn stream_id(&self) -> uuid::Uuid {
        self.inner.stream_id()
    }

    pub fn total_cost_micros(&self) -> u64 {
        self.inner.total_cost_micros()
    }

    /// UI gap #1: the session vitals snapshot an ambient status bar
    /// paints after every beat - model, mission/step/call counters,
    /// metered cost, wall time, stream id.
    /// Eric's five #4: switch the operator model from the next
    /// mission onward (None restores the config default).
    pub fn set_model_override(&mut self, model: Option<String>) -> Result<(), LoopError> {
        self.inner.set_model_override(model)
    }

    /// Configured models as (name, `is_default`) for the picker.
    /// Recovery tier B: snapshot the mission workdir into the
    /// content-addressed store (evidence: `SnapshotRef` events).
    pub fn snapshot_workdir(&mut self) -> Result<hs_world::SnapshotReport, LoopError> {
        self.inner.snapshot_workdir()
    }
    /// Recovery tier B: hash-verified, byte-exact workdir restore,
    /// booked and measured on the mission stream.
    pub fn restore_workdir(
        &mut self,
        snapshot_id: &str,
    ) -> Result<hs_world::SnapshotReport, LoopError> {
        self.inner.restore_workdir(snapshot_id)
    }

    
    pub fn model_names(&self) -> Vec<(String, bool)> {
        self.inner.model_names()
    }

    pub fn vitals(&self) -> SessionVitals {
        SessionVitals {
            model_label: self.model_label.clone(),
            missions_run: self.missions_run,
            total_steps: self.total_steps,
            total_model_calls: self.total_model_calls,
            total_cost_micros: self.total_cost_micros(),
            conservative_cost_micros: self.inner.conservative_cost_total_micros(),
            elapsed: self.started.elapsed(),
            stream_id: self.stream_id(),
        }
    }

    pub fn set_budget_micros(&mut self, micros: u64) {
        self.inner.set_budget_micros(micros);
    }

    /// The armed session budget (micro-USD) - always `Some` after
    /// construction (D4): config `[run] budget_usd`/`budget_micros` when
    /// declared, else `DEFAULT_SESSION_BUDGET_MICROS`.
    #[must_use]
    pub fn budget_micros(&self) -> Option<u64> {
        self.inner.budget_micros()
    }

    pub fn set_wall_secs(&mut self, secs: u64) {
        self.inner.set_wall_secs(secs);
    }

    /// Gap #2: operator steering inbox, drained into the mission prompt at
    /// every step boundary.
    pub fn set_steering_inbox(&mut self, path: &Path) {
        self.inner.set_steering_inbox(path);
    }

    /// B3 (v5 2.5): gateway task inbox for mid-run goal injection.
    pub fn set_task_inbox(&mut self, path: &Path) {
        self.inner.set_task_inbox(path);
    }

    /// Goals injected mid-run via the gateway task inbox, drained.
    pub fn take_queued_goals(&mut self) -> Vec<String> {
        self.inner.take_queued_goals()
    }

    /// Gap #2: operator interrupt flag - the mission stops cleanly at the
    /// next step boundary once this file exists.
    pub fn set_interrupt_file(&mut self, path: &Path) {
        self.inner.set_interrupt_file(path);
    }

    /// Gap #3: live model-output deltas (hs-repl prints them to stderr).
    pub fn set_delta_sink(&mut self, sink: hs_kernel::DeltaSink) {
        self.inner.set_delta_sink(sink);
    }

    /// UI batch 1: typed mission UI events for the REPL painter.
    pub fn set_ui_sink(&mut self, sink: crate::uipaint::UiSink) {
        self.inner.set_ui_sink(sink);
    }

    /// UI gap #4: the bin owns the markdown streamer's flush; the shared
    /// interactive loop calls it after each mission so a partial final
    /// line of model prose lands before the status bar repaints.
    pub fn set_ui_flush(&mut self, flush: Box<dyn FnMut() + Send>) {
        self.ui_flush = Some(flush);
    }

    fn flush_ui(&mut self) {
        if let Some(f) = self.ui_flush.as_mut() {
            f();
        }
    }
}

/// The shared session constructor every hs-repl mode uses (UI gap #7
/// live defect: the interactive arm silently ignored --resume/--fork and
/// opened a fresh stream; the picker's choice went nowhere). resume and
/// fork are exclusive; resume adopts the picked stream, fork branches it.
pub fn load_session(
    config: &Path,
    log_root: &Path,
    feedback: bool,
    max_steps: u32,
    resume: Option<uuid::Uuid>,
    fork: Option<uuid::Uuid>,
) -> Result<ReplSession, LoopError> {
    match (resume, fork) {
        (Some(_), Some(_)) => Err(LoopError::Visibility(
            "--resume and --fork are exclusive".to_string(),
        )),
        (Some(id), None) => ReplSession::load_resume(config, log_root, feedback, max_steps, id),
        (None, Some(parent)) => ReplSession::load_fork(config, log_root, feedback, max_steps, parent),
        (None, None) => ReplSession::load(config, log_root, feedback, max_steps),
    }
}

/// One goal, one session, end to end: load the kernel, run the mission,
/// return the result. The `hs-repl run` path.
pub fn run_one_shot(
    config: &Path,
    log_root: &Path,
    goal: &str,
    feedback: bool,
    max_steps: u32,
) -> Result<MissionResult, LoopError> {
    ReplSession::load(config, log_root, feedback, max_steps)?.run_goal(goal)
}

/// UI gap #7: the resume picker. One prior session per stream in the
/// log root, typed (no print-scraping): stream id, event count, the
/// first mission's goal as a human preview, and the last-write time.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: uuid::Uuid,
    pub events: u64,
    pub preview: String,
    pub modified: std::time::SystemTime,
}

fn truncate60(s: &str) -> String {
    if s.chars().count() <= 60 {
        return s.to_string();
    }
    let mut t: String = s.chars().take(59).collect();
    t.push('\u{2026}');
    t
}

/// Every resumable session under `log_root`, newest first. Streams that
/// fail to open or read are skipped - the picker lists what resume can
/// actually adopt.
#[must_use]
pub fn list_sessions(log_root: &Path) -> Vec<SessionInfo> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(log_root.join("streams")) else {
        return out;
    };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(id) = uuid::Uuid::parse_str(&name) else {
            continue;
        };
        let Ok(reader) = hs_log::StreamReader::open(log_root, id) else {
            continue;
        };
        let Ok(events) = reader.events() else {
            continue;
        };
        // Only OPERATOR streams are resumable sessions. A REPL run also
        // leaves an aux (journal) stream behind; the operator stream is
        // the one InnerLoop reports as MissionResult.stream_id and the
        // only one load_resume can adopt - marked by Feedback/GoalUpdate
        // events, which the aux stream never carries (live probe
        // 2026-09-08: operator kinds [0,1,5,0,5,9], aux [2,0,2,1,...]).
        if !events.iter().any(|e| {
            matches!(
                e.kind,
                hs_core::EventKind::Feedback | hs_core::EventKind::GoalUpdate
            )
        }) {
            continue;
        }
        let mut preview = String::new();
        for ev in events.iter().take(4) {
            if let Ok(bytes) = reader.resolve_payload(ev) {
                let text = String::from_utf8_lossy(&bytes);
                if let Some(i) = text.find("MISSION: ") {
                    // payload is one JSON line: cut at the closing quote
                    preview = text[i + 9..]
                        .split(['"', '\n'])
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    break;
                }
            }
        }
        // M13: the payload is one JSON line, so a multi-line goal
        // arrives as an ESCAPED two-char \n that the real-newline
        // split above never sees - flatten it (cap5 picker showed a
        // literal "\n" mid-preview).
        preview = preview.replace("\\n", " ").replace("\\t", " ");
        if preview.is_empty() {
            preview = "(no mission)".to_string();
        }
        // Directory mtime only moves on file creation; the last seg
        // file's mtime tracks the stream's actual last write.
        let mut modified = std::time::SystemTime::UNIX_EPOCH;
        if let Ok(files) = std::fs::read_dir(entry.path()) {
            for f in files.flatten() {
                if let Ok(m) = f.metadata().and_then(|md| md.modified())
                    && m > modified {
                        modified = m;
                    }
            }
        }
        out.push(SessionInfo {
            id,
            events: events.len() as u64,
            preview: truncate60(&preview),
            modified,
        });
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.id.cmp(&b.id)));
    out
}


/// M16: the picker never offers the session you are already in.
/// Recovery tier C (spec v5 "Recovery tiers" C: "VM recycled or lost -
/// full fidelity comes from the log plus [snapshots]"). Recreate the
/// mission from the durable substrate ALONE: resume the most recent
/// mission stream, restore its newest `SnapshotRef` into a fresh workdir,
/// book the recovery honestly measured (tier budgets are "reported
/// honestly", never rounded down). Returns what was recreated so the
/// caller resumes the session on `recover.stream`.
pub struct ColdRecovery {
    pub stream: uuid::Uuid,
    pub snapshot_id: Option<String>,
    pub files_restored: u64,
    pub duration_ms: u64,
}
pub fn cold_recover(run: &Path) -> Result<ColdRecovery, LoopError> {
    let t0 = std::time::Instant::now();
    let info = list_sessions(run)
        .into_iter()
        .next()
        .ok_or_else(|| LoopError::Visibility(format!("no streams under {}", run.display())))?;
    let reader = hs_log::StreamReader::open(run, info.id)?;
    let events = reader.events()?;
    let snapshot_id = events
        .iter()
        .rev()
        .filter(|e| e.kind == hs_core::EventKind::SnapshotRef)
        .find_map(|e| {
            reader
                .resolve_payload(e)
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .and_then(|v| v.get("snapshot_id")?.as_str().map(str::to_string))
        });
    let mut files_restored = 0u64;
    if let Some(id) = &snapshot_id {
        let world = hs_world::World::open(run).map_err(|e| LoopError::World(format!("{e:?}")))?;
        let rep = world
            .restore(id, &run.join("work"))
            .map_err(|e| LoopError::World(format!("{e:?}")))?;
        files_restored = rep.files;
    }
    let duration_ms = t0.elapsed().as_millis() as u64;
    let mut w = hs_log::StreamWriter::resume(run, info.id)?.writer;
    w.append(
        hs_core::EventBuilder::new(hs_core::EventKind::Observation).payload(
            hs_core::Payload::Inline(
                format!(
                    "recovery tier=C cold-recreate snapshot_id={} files={files_restored} duration_ms={duration_ms}",
                    snapshot_id.clone().unwrap_or_else(|| "none".to_string())
                )
                .into_bytes(),
            ),
        ),
    )?;
    Ok(ColdRecovery {
        stream: info.id,
        snapshot_id,
        files_restored,
        duration_ms,
    })
}

/// Resuming your own live stream would fork state mid-run; pi/omp
/// pickers never list it. Same listing as `list_sessions`, minus the
/// active stream id. A foreign id excludes nothing.
#[must_use]
pub fn list_sessions_excluding(log_root: &Path, current: uuid::Uuid) -> Vec<SessionInfo> {
    list_sessions(log_root)
        .into_iter()
        .filter(|s| s.id != current)
        .collect()
}

/// Map a picker's 1-based numeric selection to a stream id. Anything
/// else - zero, out of range, non-numeric - selects nothing.
#[must_use]
pub fn pick_session(infos: &[SessionInfo], input: &str) -> Option<uuid::Uuid> {
    let n: usize = input.trim().parse().ok()?;
    if n == 0 || n > infos.len() {
        return None;
    }
    Some(infos[n - 1].id)
}

/// One numbered picker line: short id, event count, mission preview.
#[must_use]
pub fn session_line(i: usize, info: &SessionInfo) -> String {
    let short: String = info.id.to_string().chars().take(8).collect();
    format!("{i}) {short}  {} events  {}", info.events, info.preview)
}

/// UI gap #6: tab completion for the REPL's :commands (pi/omp
/// complete their commands at the prompt; hs-repl made the operator
/// type them from memory). Pure prefix function so it is testable
/// without a TTY; `CommandCompleter` adapts it to rustyline.
pub const REPL_COMMANDS: &[&str] =
    &[":help", ":history", ":last", ":quit", ":restore", ":snapshot", ":status"];

/// Completions for a command prefix. Only colon-prefixed input
/// completes; goal text never does.
#[must_use]
pub fn command_completions(prefix: &str) -> Vec<String> {
    if !prefix.starts_with(':') {
        return Vec::new();
    }
    REPL_COMMANDS
        .iter()
        .filter(|c| c.starts_with(prefix))
        .map(std::string::ToString::to_string)
        .collect()
}

/// rustyline adapter: Tab on ":st" replaces the whole line with
/// ":status" (start = 0), so commands complete in place.
pub struct CommandCompleter;

impl rustyline::completion::Completer for CommandCompleter {
    type Candidate = rustyline::completion::Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<rustyline::completion::Pair>)> {
        let prefix = &line[..pos.min(line.len())];
        let pairs = command_completions(prefix)
            .into_iter()
            .map(|c| rustyline::completion::Pair {
                display: c.clone(),
                replacement: c,
            })
            .collect();
        Ok((0, pairs))
    }
}

impl rustyline::hint::Hinter for CommandCompleter {
    type Hint = String;
}
impl rustyline::highlight::Highlighter for CommandCompleter {}
impl rustyline::validate::Validator for CommandCompleter {}
impl rustyline::Helper for CommandCompleter {}

/// Gap #5: line editing. The interactive loop reads through an Editor:
/// a rustyline-backed TTY editor (real editing keys) or a piped-stdin
/// fallback. HAIRSPRING owns the history lifecycle: every accepted line
/// is appended to <session dir>/.`hs_repl_history` and preloaded by the
/// next session on the same dir - up-arrow works across restarts.
pub trait Editor {
    /// One line of input, None on EOF/Ctrl-C.
    fn read_line(&mut self, prompt: &str) -> std::io::Result<Option<String>>;
    fn add_history(&mut self, line: &str);
    fn history(&self) -> &[String];
}

const HISTORY_FILE: &str = ".hs_repl_history";

fn load_history(log_root: &Path) -> Vec<String> {
    std::fs::read_to_string(log_root.join(HISTORY_FILE))
        .map(|b| b.lines().map(std::string::ToString::to_string).collect())
        .unwrap_or_default()
}

fn append_history(log_root: &Path, line: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_root.join(HISTORY_FILE))
    {
        let _ = writeln!(f, "{line}");
    }
}

/// Non-TTY editor: plain lines from any `BufRead`, history still
/// recorded and preloaded. The prompt goes to stderr (stdout stays
/// clean for result JSON).
pub struct StdinEditor<R: std::io::BufRead> {
    reader: R,
    log_root: PathBuf,
    history: Vec<String>,
}

impl<R: std::io::BufRead> StdinEditor<R> {
    pub fn new(log_root: &Path, reader: R) -> Self {
        StdinEditor {
            reader,
            log_root: log_root.to_path_buf(),
            history: load_history(log_root),
        }
    }
}

impl<R: std::io::BufRead> Editor for StdinEditor<R> {
    fn read_line(&mut self, prompt: &str) -> std::io::Result<Option<String>> {
        eprint!("{prompt}");
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        Ok(Some(line))
    }
    fn add_history(&mut self, line: &str) {
        append_history(&self.log_root, line);
        self.history.push(line.to_string());
    }
    fn history(&self) -> &[String] {
        &self.history
    }
}

/// TTY editor: rustyline gives the real input line - cursor movement,
/// emacs/vi keys, kill ring - and HAIRSPRING's file history is loaded
/// into it so recall spans restarts.
pub struct RustylineEditor {
    rl: rustyline::Editor<CommandCompleter, rustyline::history::DefaultHistory>,
    log_root: PathBuf,
    history: Vec<String>,
}

impl RustylineEditor {
    pub fn new(log_root: &Path) -> Result<Self, rustyline::error::ReadlineError> {
        let mut rl = rustyline::Editor::new()?;
        // UI gap #6: Tab completes :commands at the prompt.
        rl.set_helper(Some(CommandCompleter));
        let history = load_history(log_root);
        for h in &history {
            let _ = rl.add_history_entry(h.as_str());
        }
        Ok(RustylineEditor {
            rl,
            log_root: log_root.to_path_buf(),
            history,
        })
    }
}

impl Editor for RustylineEditor {
    fn read_line(&mut self, prompt: &str) -> std::io::Result<Option<String>> {
        match self.rl.readline(prompt) {
            Ok(line) => Ok(Some(line)),
            Err(rustyline::error::ReadlineError::Interrupted |
rustyline::error::ReadlineError::Eof) => Ok(None),
            Err(e) => Err(std::io::Error::other(e)),
        }
    }
    fn add_history(&mut self, line: &str) {
        let _ = self.rl.add_history_entry(line);
        append_history(&self.log_root, line);
        self.history.push(line.to_string());
    }
    fn history(&self) -> &[String] {
        &self.history
    }
}

/// Print one mission result as JSON on stdout (the machine-readable
/// surface; prompts and chatter stay on stderr).
pub fn print_result(r: &crate::MissionResult) {
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "passed": r.passed,
            "steps": r.steps,
            "model_calls": r.model_calls,
            "outcome": r.outcome,
            "budget_killed": r.budget_killed,
            "harness_error": r.harness_error,
            "stream_id": r.stream_id.to_string(),
            "answer_path": r.answer_path.display().to_string(),
        }))
        .expect("json! values serialize")
    );
}

/// UI gap #1: paint the ambient status bar to stderr (colored on a
/// terminal, plain when piped).
fn paint_status(session: &ReplSession) {
    use std::io::IsTerminal;
    let color = std::io::stderr().is_terminal();
    let mut err = std::io::stderr();
    // UI gap #9: the status bar takes the operator's theme (HS_THEME).
    let theme = crate::uipaint::Theme::from_env();
    let mut p = crate::uipaint::Painter::with_theme(&mut err, color, &theme);
    p.status_line(&session.vitals());
}

/// The interactive loop: read a line, record it, dispatch. Shared by
/// the TTY and piped paths so behavior is identical on both.
pub fn run_interactive<E: Editor + ?Sized>(
    session: &mut ReplSession,
    editor: &mut E,
) -> Result<(), Box<dyn std::error::Error>> {
    // UI gap #1: the ambient status bar. Painted on session start and
    // after every mission, so the operator never types :status to learn
    // where the session stands. stderr only - stdout stays clean JSON.
    paint_status(session);
    // UI gap #8: the composer frame. TTY only - piped stdin keeps the
    // byte-plain "hs> " prompt and zero chrome so scripts never see
    // box glyphs. Width follows COLUMNS, bounded sanely.
    let color = {
        use std::io::IsTerminal;
        std::io::stderr().is_terminal()
    };
    let cols: usize = std::env::var("COLUMNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(72, |c: usize| c.clamp(40, 120));
    let prompt = if color { crate::uipaint::EDITOR_PROMPT } else { "hs> " };
    let theme = crate::uipaint::Theme::from_env();
    loop {
        if color {
            let v = session.vitals();
            eprintln!(
                "{}",
                crate::uipaint::composer_top_themed(
                    &format!("{} \u{00b7} {}", v.model_label, crate::uipaint::format_usd_micros(v.total_cost_micros)),
                    cols,
                    true,
                    &theme,
                )
            );
        }
        let line = match editor.read_line(prompt)? {
            None => break,
            Some(l) => l.trim().to_string(),
        };
        if color {
            eprintln!("{}", crate::uipaint::composer_bottom(cols, true));
        }
        if line.is_empty() {
            continue;
        }
        editor.add_history(&line);
        match parse_command(&line) {
            ReplCommand::Quit => break,
            ReplCommand::Help => eprintln!("{REPL_HELP}"),
            ReplCommand::Status => {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "stream_id": session.stream_id().to_string(),
                        "cost_usd_micros": session.total_cost_micros(),
                    }))
                    .expect("json! values serialize")
                );
            }
            ReplCommand::History => {
                for h in editor.history() {
                    println!("{h}");
                }
            }
            ReplCommand::LastAnswer => match session.last_answer() {
                Some(a) => println!("{a}"),
                None => eprintln!("no mission has run yet"),
            },
            ReplCommand::Snapshot => match session.snapshot_workdir() {
                Ok(r) => println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "snapshot_id": r.snapshot_id, "files": r.files, "bytes": r.bytes,
                    }))
                    .expect("json! values serialize")
                ),
                Err(e) => eprintln!("snapshot failed: {e}"),
            },
            ReplCommand::Restore(id) => match session.restore_workdir(&id) {
                Ok(r) => println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "restored": r.snapshot_id, "files": r.files, "bytes": r.bytes,
                    }))
                    .expect("json! values serialize")
                ),
                Err(e) => eprintln!("restore failed: {e}"),
            },
            ReplCommand::Unknown(c) => {
                eprintln!("unknown command {c} (:help lists commands)");
            }
            ReplCommand::Goal(goal) => {
                match session.run_goal(&goal) {
                    Ok(r) => print_result(&r),
                    Err(e) => eprintln!("mission failed: {e}"),
                }
                // B3: gateway adds queued mid-run execute after the close.
                for queued in session.take_queued_goals() {
                    match session.run_goal(&queued) {
                        Ok(r) => print_result(&r),
                        Err(e) => eprintln!("queued mission failed: {e}"),
                    }
                }
                session.flush_ui();
                paint_status(session);
            }
        }
    }
    Ok(())
}
