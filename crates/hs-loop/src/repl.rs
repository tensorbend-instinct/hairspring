//! REPL surface (Eric 2026-09-07): run the HAIRSPRING harness end-to-end on
//! a goal from the CLI - one-shot (`hs-repl run --goal ...`) or interactive
//! (`hs-repl`, one goal per line, `:`-prefixed commands).
//!
//! This module is the testable core; the bin is a thin stdin/stdout shell
//! over it. Everything here reuses the production machinery: swe_kernel +
//! InnerLoop, require_visibility gate included.

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
    /// Print the latest mission's answer artifact.
    LastAnswer,
    Quit,
    /// `:`-prefixed input that names no command.
    Unknown(String),
}

pub fn parse_command(line: &str) -> ReplCommand {
    let t = line.trim();
    match t {
        ":quit" | ":q" | ":exit" => ReplCommand::Quit,
        ":help" | ":h" | ":?" => ReplCommand::Help,
        ":status" => ReplCommand::Status,
        ":last" => ReplCommand::LastAnswer,
        _ if t.starts_with(':') => ReplCommand::Unknown(t.to_string()),
        _ => ReplCommand::Goal(t.to_string()),
    }
}

/// Path-safe mission id from goal text: lowercase, alnum runs joined by
/// single dashes, capped at 40 chars. The id names the work dir
/// (log_root/work/<id>/answer.txt), so it must never contain separators.
pub fn goal_slug(goal: &str) -> String {
    let mut out = String::with_capacity(goal.len().min(41));
    let mut dash = false;
    for c in goal.chars().flat_map(|c| c.to_lowercase()) {
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
  :last     print the latest mission's answer artifact
  :help     this text
  :quit     exit";

/// A loaded kernel + loop pair that runs goals as missions, in order.
pub struct ReplSession {
    inner: InnerLoop,
    used_ids: std::collections::HashSet<String>,
    last_answer_path: Option<PathBuf>,
    mcp_catalog: String,
}

impl ReplSession {
    /// Load the kernel from `config`, gated by require_visibility (the
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
        let mut native_tools = crate::toolschema::tb_tools();
        let merged_config;
        let config = if let Ok(servers_toml) = std::env::var("HS_MCP_SERVERS") {
            let (fragment, native) =
                crate::mcpbridge::discover_mcp_tools(std::path::Path::new(&servers_toml))
                    .map_err(LoopError::Visibility)?;
            native_tools.extend(native.iter().cloned());
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
        let kernel = swe_kernel(config, log_root)?;
        require_visibility(&kernel).map_err(LoopError::Visibility)?;
        let mut inner = InnerLoop::new(kernel, log_root, feedback, max_steps)?;
        inner.set_tools(serde_json::Value::Array(native_tools));
        Ok(ReplSession {
            inner,
            used_ids: std::collections::HashSet::new(),
            last_answer_path: None,
            mcp_catalog,
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
        let id = self.mission_id_for(goal);
        let prompt = if self.mcp_catalog.is_empty() {
            goal.to_string()
        } else {
            format!("{goal}\n\nAVAILABLE MCP TOOLS (call them like any other tool):\n{}", self.mcp_catalog)
        };
        let r = self.inner.run_mission_full(&id, &prompt)?;
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

    pub fn set_budget_micros(&mut self, micros: u64) {
        self.inner.set_budget_micros(micros);
    }

    pub fn set_wall_secs(&mut self, secs: u64) {
        self.inner.set_wall_secs(secs);
    }

    /// Gap #2: operator steering inbox, drained into the mission prompt at
    /// every step boundary.
    pub fn set_steering_inbox(&mut self, path: &Path) {
        self.inner.set_steering_inbox(path);
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
