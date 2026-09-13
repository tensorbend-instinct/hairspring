//! REPL surface (Eric 2026-09-07): run the HAIRSPRING harness end-to-end on
//! a goal from the CLI - one-shot (`hs-repl run --goal ...`) or interactive
//! (`hs-repl`, one goal per line, `:`-prefixed commands).
//!
//! This module is the testable core; the bin is a thin stdin/stdout shell
//! over it. Everything here reuses the production machinery: `swe_kernel` +
//! `InnerLoop`, `require_visibility` gate included.

use crate::{require_visibility, swe_kernel, swe_kernel_lenient, InnerLoop, LoopError, MissionResult};
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
    // Sigil normalization: "/cmd" is the canonical spelling; ":cmd"
    // is a backward-compatible alias (Eric 2026-09-10).
    let owned;
    let t = if let Some(rest) = t.strip_prefix('/') {
        owned = format!(":{rest}");
        &owned
    } else {
        t
    };
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
/// Mission work-anchor resolution (isolation: the project directory is a
/// mission-start input). `hs-repl --project-dir` exports HS_PROJECT_ROOT
/// after startup validation + canonicalization; the mission work anchor IS
/// that root. Without it the anchor stays `<log_root>/work`, which
/// `wire_tool_env` also exports as the default confinement root - every
/// TUI session is confined by default. A root that no longer canonicalizes
/// is a startup-integrity failure: refuse loudly, never anchor silently.
pub fn resolve_work_dir(log_root: &std::path::Path) -> std::path::PathBuf {
    match std::env::var("HS_PROJECT_ROOT") {
        Ok(raw) if !raw.is_empty() => std::fs::canonicalize(&raw).unwrap_or_else(|e| {
            panic!(
                "HS_PROJECT_ROOT {raw} cannot be canonicalized: {e} - refusing to anchor missions on an unverifiable root"
            )
        }),
        _ => log_root.join("work"),
    }
}

pub struct ReplSession {
    inner: InnerLoop,
    used_ids: std::collections::HashSet<String>,
    last_answer_path: Option<PathBuf>,
    mcp_catalog: String,
    /// The promoted policy overlay this session runs under (checklist 6.9):
    /// None = builtin behavior (the goal IS the prompt, dance #95).
    policy: Option<crate::sweprompt::PolicyOverlay>,
    model_label: String,
    started: std::time::Instant,
    missions_run: u64,
    total_steps: u64,
    total_model_calls: u64,
    ui_flush: Option<Box<dyn FnMut() + Send>>,
    work_dir: PathBuf,
    config_path: PathBuf,
}

/// The promoted policy overlay the live session runs under (checklist 6.9:
/// promotion -> config -> live session). HS_POLICY_TOML wins, else the
/// canonical config-dir overlay when it exists; absent = builtin behavior.
/// Malformed is a hard error, never a silent fallback (sweprompt law).
#[must_use]
pub fn load_tui_policy() -> Option<crate::sweprompt::PolicyOverlay> {
    let path = crate::sweprompt::resolve_policy_overlay_path()?;
    match crate::sweprompt::load_policy_overlay(&path) {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("hairspring: {e}");
            std::process::exit(2);
        }
    }
}

/// Offered-surface dedupe (live 400, 2026-09-10): the config may itself
/// register agent.spawn/agent.spawn_poll (the shipped rig does) while the
/// interactive surface offers its own pair - the registry-derived list
/// plus the unconditional pushes produced DUPLICATE wire names and
/// DeepSeek rejected every live mission ("Tool names must be unique").
/// Every offered-extra goes through here: offered == registered, once.
fn offer_unique(native_tools: &mut Vec<serde_json::Value>, t: serde_json::Value) {
    let name = t["function"]["name"].as_str().unwrap_or("").to_string();
    if !native_tools
        .iter()
        .any(|x| x["function"]["name"].as_str() == Some(name.as_str()))
    {
        native_tools.push(t);
    }
}

    /// Eric 2026-09-10: /caps persists to the config file; a restarted
/// session arms what the file holds. `[run] max_steps = <n>`.
#[must_use]
pub fn configured_max_steps(config: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(config).ok()?;
    let v: toml::Value = toml::from_str(&text).ok()?;
    v.get("run")?
        .as_table()?
        .get("max_steps")?
        .as_integer()
        .and_then(|n| u32::try_from(n.max(1)).ok())
}

/// `[run] wall_secs = <n>` (absent or removed = no wall cap).
#[must_use]
pub fn configured_wall_secs(config: &Path) -> Option<u64> {
    let text = std::fs::read_to_string(config).ok()?;
    let v: toml::Value = toml::from_str(&text).ok()?;
    v.get("run")?
        .as_table()?
        .get("wall_secs")?
        .as_integer()
        .map(|n| n.max(0) as u64)
}

/// `[critic] max_steps / wall_secs / budget_micros` - persisted critic
/// caps; armed as env at load so the spawned critic plugin inherits
/// them (RefuteConfig::from_env stays the single reader).
#[must_use]
pub fn configured_critic_caps(config: &Path) -> (Option<u64>, Option<u64>, Option<u64>) {
    let get = |k: &str| -> Option<u64> {
        let text = std::fs::read_to_string(config).ok()?;
        let v: toml::Value = toml::from_str(&text).ok()?;
        v.get("critic")?
            .as_table()?
            .get(k)?
            .as_integer()
            .map(|n| n.max(0) as u64)
    };
    (get("max_steps"), get("wall_secs"), get("budget_micros"))
}

/// Flip the active default across the config's [[models]] blocks
/// (Eric 2026-09-10: a /models pick must survive restart - same gap
/// class as /caps). The picked block gains `default = true`, every
/// other block loses its ACTIVE default; commented-out defaults are
/// text, not config, and stay untouched. Unknown name is an error
/// naming the known set.
pub fn set_default_model_text(text: &str, name: &str) -> Result<String, String> {
    let lines: Vec<&str> = text.lines().collect();
    // Block spans: each [[models]] header to the next [ section header.
    let mut blocks: Vec<(usize, usize)> = Vec::new(); // [header, end)
    let mut cur: Option<usize> = None;
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim();
        if t.starts_with('[') {
            if let Some(h) = cur.take() {
                blocks.push((h, i));
            }
            if t == "[[models]]" {
                cur = Some(i);
            }
        }
    }
    if let Some(h) = cur {
        blocks.push((h, lines.len()));
    }
    let name_line = |i: usize| -> Option<String> {
        let t = lines[i].trim();
        if t.starts_with("name") {
            if let Some(eq) = t.find('=') {
                return Some(
                    t[eq + 1..]
                        .trim()
                        .trim_matches('"')
                        .to_string(),
                );
            }
        }
        None
    };
    let mut known: Vec<String> = Vec::new();
    let mut target: Option<(usize, usize)> = None;
    for b in &blocks {
        for i in b.0..b.1 {
            if let Some(n) = name_line(i) {
                if n == name {
                    target = Some(*b);
                }
                known.push(n);
                break;
            }
        }
    }
    let Some(tb) = target else {
        return Err(format!(
            "unknown model {name:?} (known: {})",
            known.join(", ")
        ));
    };
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 1);
    let mut inserted = false;
    for (i, l) in lines.iter().enumerate() {
        let in_block = blocks.iter().any(|b| b.0 <= i && i < b.1);
        if in_block && l.trim() == "default = true" {
            continue; // every active default goes; the target re-adds its own
        }
        out.push((*l).to_string());
        if !inserted && tb.0 <= i && i < tb.1 && name_line(i).as_deref() == Some(name) {
            out.push("default = true".to_string());
            inserted = true;
        }
    }
    let mut joined = out.join("\n");
    if text.ends_with('\n') && !joined.ends_with('\n') {
        joined.push('\n');
    }
    Ok(joined)
}

/// Persist a /models pick: flip the default in the config file.
/// Applied live by the caller only after this write lands.
pub fn set_default_model(config: &Path, name: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(config)
        .map_err(|e| format!("read {}: {e}", config.display()))?;
    let out = set_default_model_text(&text, name)?;
    std::fs::write(config, out).map_err(|e| format!("write {}: {e}", config.display()))
}

/// One key in one TOML section: created when missing, replaced in
/// place when present, removed when `value` is None. Everything else
/// in the file (comments, other keys, other sections) is preserved
/// byte-for-byte. Flat scalars only - the caps writer's whole job.
pub fn toml_upsert(
    text: &str,
    section: &str,
    key: &str,
    value: Option<&str>,
) -> Result<String, String> {
    if section.is_empty()
        || key.is_empty()
        || section.contains(['[', ']', '\n'])
        || key.contains(['=', '\n'])
    {
        return Err(format!("bad section/key for toml_upsert: [{section}] {key}"));
    }
    let header = format!("[{section}]");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    match lines.iter().position(|l| l.trim() == header) {
        Some(h) => {
            let mut found = None;
            let mut i = h + 1;
            while i < lines.len() {
                let t = lines[i].trim();
                if t.starts_with('[') {
                    break;
                }
                if let Some(eq) = t.find('=')
                    && t[..eq].trim() == key
                {
                    found = Some(i);
                    break;
                }
                i += 1;
            }
            match (found, value) {
                (Some(i), Some(v)) => lines[i] = format!("{key} = {v}"),
                (Some(i), None) => {
                    lines.remove(i);
                }
                (None, Some(v)) => lines.insert(h + 1, format!("{key} = {v}")),
                (None, None) => {}
            }
        }
        None => {
            if let Some(v) = value {
                if lines.last().is_some_and(|l| !l.is_empty()) {
                    lines.push(String::new());
                }
                lines.push(header);
                lines.push(format!("{key} = {v}"));
            }
        }
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') && !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}


/// Canonical TOML literal for a dollar figure: ALWAYS a float, so the
/// reader above never sees the integer form a bare `format!("{dollars}")`
/// produced for whole dollars (the $100 -> $10 bug).
#[must_use]
pub fn usd_literal(micros: u64) -> String {
    let dollars = micros as f64 / 1e6;
    let t = format!("{dollars:.6}");
    let t = t.trim_end_matches('0');
    let t = t.strip_suffix('.').map_or(t, |x| x);
    if t.contains('.') {
        t.to_string()
    } else {
        format!("{t}.0")
    }
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
pub fn configured_model_label(config: &Path) -> Option<String> {
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

/// The run's USD budget from the config, when declared
/// (`[run] budget_usd = <dollars>` or `budget_micros = <micros>`;
/// dollars win when both appear). Missing means NO budget cap is armed
/// (Eric 2026-09-12: caps are opt-in). `budget_usd` accepts a float OR
/// an integer literal: /caps once wrote whole dollars as a TOML integer
/// and the float-only reader silently dropped them - Eric's $100 came
/// back as the hidden $10 default (2026-09-12).
#[must_use]
pub fn configured_budget_micros(config: &Path) -> Option<u64> {
    let text = std::fs::read_to_string(config).ok()?;
    let v: toml::Value = toml::from_str(&text).ok()?;
    let run = v.get("run")?.as_table()?;
    let usd = run.get("budget_usd").and_then(|v| {
        v.as_float().or_else(|| v.as_integer().map(|i| i as f64))
    });
    if let Some(usd) = usd {
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
    fn wire_tool_env(log_root: &Path, config: &Path) -> Result<(), LoopError> {
        std::fs::create_dir_all(log_root)?;
        // D6 (live burn 2026-09-09): anchor the repo tools at the
        // session's WORK area, never the run root - the root holds
        // harness state (streams/, blobs/, memory.db) the mission must
        // not read through repo.read/repo.search.
        // Isolation (2026-09-10): the anchor honors HS_PROJECT_ROOT when
        // the operator passed --project-dir; otherwise it is the session
        // work area. Both cases export the root below, so confinement is
        // always on for TUI sessions.
        let work = resolve_work_dir(log_root);
        std::fs::create_dir_all(&work)?;
        // D10 (live burn 2026-09-09, realrun3): HS_SELFCHECK_DIRECT binds
        // the checker to the REGISTERED surface, never unconditionally.
        // Live-machine surface (term.exec): the agent works the real
        // workdir, so .hs/checks lives there - DIRECT=1. Candidate
        // surface (edit.patch/edit.anchor/edit.apply): the agent writes
        // into the candidate worktree, so the checker must look there -
        // DIRECT unset. Direct was set unconditially for the tb rig and
        // silently ungreened every candidate-surface REPL mission
        // ("no checks declared" against run/work on a correct mission).
        let text = std::fs::read_to_string(config)?;
        let parsed: toml::Value = toml::from_str(&text).map_err(|e| LoopError::Visibility(e.to_string()))?;
        let names: Vec<&str> = parsed
            .get("tools")
            .and_then(toml::Value::as_array)
            .map(|ts| {
                ts.iter()
                    .filter_map(|t| t.get("name").and_then(toml::Value::as_str))
                    .collect()
            })
            .unwrap_or_default();
        let live_machine = names.contains(&"term.exec");
        let candidate_surface =
            names.iter().any(|n| matches!(*n, "edit.patch" | "edit.anchor" | "edit.apply"));
        unsafe {
            std::env::set_var("HS_TERM_WORKDIR", &work);
            std::env::set_var("HS_SWE_WORKSPACE", &work);
            // The EFFECTIVE confinement root for this session's plugins:
            // the operator's --project-dir when present, else the session
            // work area. Written on every session load (kernels spawn
            // long-lived plugins), so a later session in the same process
            // never inherits a stale root - suite-15 caught exactly that
            // (a deleted prior work dir panicking session two's anchor).
            let effective_root = std::env::var("HS_PROJECT_ROOT")
                .ok()
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| work.clone());
            std::env::set_var("HS_PROJECT_ROOT_EFFECTIVE", &effective_root);
            if live_machine {
                std::env::set_var("HS_SELFCHECK_DIRECT", "1");
                // Stranger-path burn (2026-09-09, run dir /tmp/hs-demo): on
                // the live surface the submission is the machine state plus
                // the agent's summary (tb semantics) - without
                // HS_ANSWER_RAW the answersubmit plugin demanded a
                // candidate-worktree diff ("make your fix with edit.patch
                // first", a tool this surface does not register) and every
                // live-surface mission ran to steps_exhausted.
                std::env::set_var("HS_ANSWER_RAW", "1");
            } else if candidate_surface {
                std::env::remove_var("HS_SELFCHECK_DIRECT");
                std::env::remove_var("HS_ANSWER_RAW");
            }
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
        max_steps: Option<u32>,
    ) -> Result<Self, LoopError> {
        Self::load_inner(config, log_root, feedback, max_steps, false)
    }

    /// Zero-config TUI stranger path (Eric 2026-09-12): the first-run TUI
    /// opens even when the default model has no credential - `/models add`
    /// fixes it in place. One-shot runs keep the hard preflight gate.
    pub fn load_lenient(
        config: &Path,
        log_root: &Path,
        feedback: bool,
        max_steps: Option<u32>,
    ) -> Result<Self, LoopError> {
        Self::load_inner(config, log_root, feedback, max_steps, true)
    }

    fn load_inner(
        config: &Path,
        log_root: &Path,
        feedback: bool,
        max_steps: Option<u32>,
        lenient: bool,
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
        Self::wire_tool_env(log_root, config)?;
        let kernel = if lenient {
            swe_kernel_lenient(config, log_root)?
        } else {
            swe_kernel(config, log_root)?
        };
        require_visibility(&kernel).map_err(LoopError::Visibility)?;
        let registered: Vec<String> = kernel
            .list_tools("operator")
            .into_iter()
            .map(|e| e.name)
            .collect();
        let mut native_tools =
            crate::toolschema::schemas_for_registry(&registered, &mcp_native, "applypatch");
        // Audit artifact, parity with hs-swe-run's tools.json: the exact
        // native tool surface the mission model operates under.
        std::fs::write(
            log_root.join("tools.json"),
            serde_json::to_string_pretty(&native_tools).expect("tools serialize"),
        )
        .expect("tools.json");
        // Eric's five #5: the interactive surface offers delegation.
        offer_unique(&mut native_tools, crate::toolschema::agent_spawn_tool());
        offer_unique(&mut native_tools, crate::toolschema::agent_spawn_poll_tool());
        let mut inner =
            InnerLoop::new(kernel, log_root, feedback, max_steps.unwrap_or(crate::DEFAULT_MISSION_MAX_STEPS))?;
        Self::apply_config_caps(&mut inner, config, max_steps);
        // Eric 2026-09-12 (live mission c91e8de3): user-facing budgets
        // bind REAL provider-reported dollars - his $40 cap killed at
        // $3.44 billed because the guard read the list-rate counter.
        // Benchmark binaries keep Conservative for ledger comparability.
        inner.set_budget_guard_mode(crate::BudgetGuardMode::ProviderReported);
        if let Some(tokens) = Self::configured_context_tokens(config) {
            inner.set_context_budget_tokens(tokens * 3 / 4);
        }
        // Eric 2026-09-12: no budget cap unless declared (config or
        // --budget-micros) - the hidden $10 default is gone.
        if let Some(b) = Self::configured_budget_micros(config) {
            inner.set_budget_micros(b);
        }
        // B1 (v5 D3): every REPL session owns the shared K plane at
        // <dir>/memory.db; the model consults it via the memory.recall
        // tool (cut #10: consulted, never pre-passed) and every mission
        // close distills into it, so each session compounds. If the db
        // cannot be created the tool simply is not offered (fail-open).
        let memory_db = log_root.join("memory.db");
        if hs_memory::sqlite::SqliteMemoryStore::open(&memory_db).is_ok() {
            offer_unique(&mut native_tools, crate::toolschema::memory_recall_tool());
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
        let (missions_run, total_steps, total_model_calls) = (0u64, 0u64, 0u64);
        Ok(ReplSession {
            inner,
            used_ids: std::collections::HashSet::new(),
            last_answer_path: None,
            mcp_catalog,
            policy: load_tui_policy(),
            model_label,
            started: std::time::Instant::now(),
            missions_run,
            total_steps,
            total_model_calls,
            ui_flush: None,
            work_dir: resolve_work_dir(&log_root),
            config_path: config.to_path_buf(),
        })
    }

    /// Gap #4 (fork): branch an existing stream into a new linked stream
    /// carrying the parent's full transcript (`hs_log::StreamWriter::fork`),
    /// then run missions on the branch. The parent stream is untouched.
    pub fn load_fork(
        config: &Path,
        log_root: &Path,
        feedback: bool,
        max_steps: Option<u32>,
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
        max_steps: Option<u32>,
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
        Self::wire_tool_env(log_root, config)?;
        let kernel = swe_kernel(config, log_root)?;
        require_visibility(&kernel).map_err(LoopError::Visibility)?;
        let registered: Vec<String> = kernel
            .list_tools("operator")
            .into_iter()
            .map(|e| e.name)
            .collect();
        let mut native_tools =
            crate::toolschema::schemas_for_registry(&registered, &mcp_native, "applypatch");
        // Audit artifact, parity with hs-swe-run's tools.json: the exact
        // native tool surface the mission model operates under.
        std::fs::write(
            log_root.join("tools.json"),
            serde_json::to_string_pretty(&native_tools).expect("tools serialize"),
        )
        .expect("tools.json");
        // Eric's five #5: the interactive surface offers delegation.
        offer_unique(&mut native_tools, crate::toolschema::agent_spawn_tool());
        offer_unique(&mut native_tools, crate::toolschema::agent_spawn_poll_tool());
        let mut inner = InnerLoop::with_stream(
            kernel,
            log_root,
            stream_id,
            feedback,
            max_steps.unwrap_or(crate::DEFAULT_MISSION_MAX_STEPS),
        )?;
        Self::apply_config_caps(&mut inner, config, max_steps);
        // Eric 2026-09-12 (live mission c91e8de3): user-facing budgets
        // bind REAL provider-reported dollars - his $40 cap killed at
        // $3.44 billed because the guard read the list-rate counter.
        // Benchmark binaries keep Conservative for ledger comparability.
        inner.set_budget_guard_mode(crate::BudgetGuardMode::ProviderReported);
        if let Some(tokens) = Self::configured_context_tokens(config) {
            inner.set_context_budget_tokens(tokens * 3 / 4);
        }
        // Eric 2026-09-12: no budget cap unless declared (config or
        // --budget-micros) - the hidden $10 default is gone.
        if let Some(b) = Self::configured_budget_micros(config) {
            inner.set_budget_micros(b);
        }
        // B1 (v5 D3): every REPL session owns the shared K plane at
        // <dir>/memory.db; the model consults it via the memory.recall
        // tool (cut #10: consulted, never pre-passed) and every mission
        // close distills into it, so each session compounds. If the db
        // cannot be created the tool simply is not offered (fail-open).
        let memory_db = log_root.join("memory.db");
        if hs_memory::sqlite::SqliteMemoryStore::open(&memory_db).is_ok() {
            offer_unique(&mut native_tools, crate::toolschema::memory_recall_tool());
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
        // Accounting reset fix (Eric 2026-09-10): fold the adopted
        // stream's history into the session counters so :resume restores
        // the accounting surfaces (HUD totals, :status cost, the vitals
        // behind the panels) instead of reading zeros. Semantics mirror
        // the live loop: one GoalUpdate per closed mission; steps are
        // loop iterations (one booked ModelCall each, excluding
        // autocompact distills, which book a call but no step); the cost
        // totals themselves are folded inside InnerLoop::with_stream.
        let mut missions_run = 0u64;
        let mut total_steps = 0u64;
        let mut total_model_calls = 0u64;
        if let Ok(r) = hs_log::StreamReader::open(log_root, stream_id) {
            if let Ok(events) = r.events() {
                for e in &events {
                    match e.kind {
                        hs_core::EventKind::GoalUpdate => missions_run += 1,
                        hs_core::EventKind::ModelCall => {
                            total_model_calls += 1;
                            let meta = r.resolve_payload(e).ok().and_then(|b| {
                                serde_json::from_slice::<serde_json::Value>(&b).ok()
                            });
                            let get = {
                                let meta = meta.clone();
                                move |k: &str| {
                                    meta.as_ref()
                                        .and_then(|v| v.get(k))
                                        .and_then(|w| w.as_str())
                                        .map(str::to_owned)
                                }
                            };
                            // distill and verifier rounds book a model call
                            // (and its cost) but no loop step - mirror that.
                            let no_step = get("why").as_deref() == Some("distill")
                                || get("role").as_deref() == Some("verifier");
                            if !no_step {
                                total_steps += 1;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(ReplSession {
            inner,
            used_ids: std::collections::HashSet::new(),
            last_answer_path: None,
            mcp_catalog,
            policy: load_tui_policy(),
            model_label,
            started: std::time::Instant::now(),
            missions_run,
            total_steps,
            total_model_calls,
            ui_flush: None,
            work_dir: resolve_work_dir(&log_root),
            config_path: config.to_path_buf(),
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
        // The mission prompt is POLICY (checklist 6.9): default is the
        // goal verbatim; a promoted [prompts] tui-mission overlay wraps it.
        let prompt = crate::sweprompt::build_tui_mission_prompt(
            self.policy.as_ref(),
            goal,
            &self.mcp_catalog,
        );
        // Burn-down (critic on the default path): the independent critic
        // gate refutes against the mission's INSTRUCTION. The tb rig
        // hands it over via HS_TB_INSTRUCTION_FILE; a TUI session is
        // long-lived with one plugin process, so the anchor lives at
        // <work>/.hs/instruction.txt, rewritten at every mission start.
        let hs_dir = self.work_dir.join(".hs");
        std::fs::create_dir_all(&hs_dir)?;
        std::fs::write(hs_dir.join("instruction.txt"), &prompt)?;
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

    /// Pick up rig changes (the /models add write) without a restart.
    pub fn reload_config(&mut self) -> Result<bool, String> {
        let r = self.inner.reload_config();
        // A fresh promotion must reach the live session too (6.9).
        self.policy = load_tui_policy();
        r
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

    /// Config-persisted caps (Eric 2026-09-10: /caps writes them, so a
    /// restart keeps them). `[run] max_steps` yields to an explicit
    /// CLI --max-steps: it applies only when the caller passed the
    /// CLI default. `[critic]` caps arm as env - the critic plugin is a
    /// spawned process whose RefuteConfig::from_env stays the reader.
    /// Eric 2026-09-12: caps are opt-in. An explicit --max-steps wins;
    /// else a persisted `[run] max_steps`; else NO step cap is armed.
    fn apply_config_caps(inner: &mut InnerLoop, config: &Path, max_steps: Option<u32>) {
        match max_steps.or_else(|| configured_max_steps(config)) {
            Some(n) => inner.set_max_steps(n),
            None => inner.clear_max_steps(),
        }
        if let Some(w) = configured_wall_secs(config) {
            inner.set_wall_secs(w);
        }
        let (cs, cw, cb) = configured_critic_caps(config);
        // SAFETY: session construction, before any mission thread or
        // critic spawn; single-threaded command dispatch otherwise.
        unsafe {
            if let Some(n) = cs {
                std::env::set_var("HS_CRITIC_MAX_STEPS", n.to_string());
            }
            if let Some(n) = cw {
                std::env::set_var("HS_CRITIC_WALL_SECS", n.to_string());
            }
            if let Some(n) = cb {
                std::env::set_var("HS_CRITIC_BUDGET_MICROS", n.to_string());
            }
        }
    }

    /// Every cap the config holds, snapshotted for /caps (Eric
    /// 2026-09-10). Critic caps read env-over-default like the critic
    /// itself, so the listing is what the next refute actually arms.
    #[must_use]
    pub fn caps_snapshot(&self) -> crate::tui::CapsSnapshot {
        let c = crate::critic::RefuteConfig::from_env();
        crate::tui::CapsSnapshot {
            steps: self.inner.max_steps(),
            wall_secs: self.inner.wall_secs(),
            budget_micros: self.inner.budget_micros(),
            critic_steps: c.max_steps,
            critic_wall_secs: c.wall_secs,
            critic_budget_micros: c.budget_micros,
        }
    }

    /// Eric 2026-09-10 (/caps): change one live cap mid-session.
    /// Operator caps apply from the next mission (a running loop keeps
    /// its range); critic caps are env the critic reads at spawn, so
    /// they bind the next refute.
    pub fn set_cap(&mut self, key: &str, value: &str) -> Result<String, String> {
        // Eric 2026-09-10: a cap change PERSISTS to the config file
        // (a setting that silently resets on restart is a gap).
        // Persist first - a failed write changes nothing live.
        self.persist_cap(key, value)?;
        let n_of = |what: &str| -> Result<u64, String> {
            value
                .parse::<u64>()
                .map_err(|_| format!("{what} needs a number, got '{value}'"))
        };
        let usd_of = |what: &str| -> Result<u64, String> {
            value
                .trim_start_matches('$')
                .parse::<f64>()
                .map(|d| (d * 1_000_000.0) as u64)
                .map_err(|_| format!("{what} needs dollars, got '{value}'"))
        };
        match key {
            "steps" => {
                if value == "off" {
                    self.inner.clear_max_steps();
                    Ok("steps \u{203a} off (no step cap)".to_string())
                } else {
                    let n = n_of("steps")?;
                    self.inner.set_max_steps(n as u32);
                    Ok(format!("steps \u{203a} {n} (next mission onward)"))
                }
            }
            "wall" => {
                if value == "off" {
                    self.inner.clear_wall_secs();
                    Ok("wall \u{203a} off".to_string())
                } else {
                    let n = n_of("wall")?;
                    self.inner.set_wall_secs(n);
                    Ok(format!("wall \u{203a} {n}s (next mission onward)"))
                }
            }
            "budget" => {
                if value == "off" {
                    self.inner.clear_budget_micros();
                    Ok("budget \u{203a} off (no spend cap)".to_string())
                } else {
                    let m = usd_of("budget")?;
                    self.inner.set_budget_micros(m);
                    Ok(format!(
                        "budget \u{203a} {}",
                        crate::uipaint::format_usd_micros(m)
                    ))
                }
            }
            "critic-steps" => {
                let n = n_of("critic-steps")?;
                // SAFETY: set between missions from the single command
                // worker; the critic reads this env once at spawn.
                unsafe { std::env::set_var("HS_CRITIC_MAX_STEPS", n.to_string()) };
                Ok(format!("critic steps \u{203a} {n}"))
            }
            "critic-wall" => {
                let n = n_of("critic-wall")?;
                // SAFETY: as above.
                unsafe { std::env::set_var("HS_CRITIC_WALL_SECS", n.to_string()) };
                Ok(format!("critic wall \u{203a} {n}s"))
            }
            "critic-budget" => {
                let m = usd_of("critic-budget")?;
                // SAFETY: as above.
                unsafe { std::env::set_var("HS_CRITIC_BUDGET_MICROS", m.to_string()) };
                Ok(format!(
                    "critic budget \u{203a} {}",
                    crate::uipaint::format_usd_micros(m)
                ))
            }
            other => Err(format!(
                "unknown cap '{other}' - keys: steps, wall, budget, critic-steps, critic-wall, critic-budget"
            )),
        }
    }

    /// Write one cap change into the session's config file. Budget
    /// lands as `budget_usd` (the reader prefers it) and any stale
    /// `budget_micros` line goes away so the file tells one truth.
    fn persist_cap(&self, key: &str, value: &str) -> Result<(), String> {
        let text = std::fs::read_to_string(&self.config_path)
            .map_err(|e| format!("read {}: {e}", self.config_path.display()))?;
        let ups = |t: &str, sec: &str, k: &str, v: Option<String>| {
            toml_upsert(t, sec, k, v.as_deref())
        };
        let out = match key {
            "steps" => {
                if value == "off" {
                    ups(&text, "run", "max_steps", None)?
                } else {
                    let n: u64 = value
                        .parse()
                        .map_err(|_| format!("steps needs a number (or off), got '{value}'"))?;
                    ups(&text, "run", "max_steps", Some(n.to_string()))?
                }
            }
            "wall" => {
                if value == "off" {
                    ups(&text, "run", "wall_secs", None)?
                } else {
                    let n: u64 = value
                        .parse()
                        .map_err(|_| format!("wall needs seconds, got '{value}'"))?;
                    ups(&text, "run", "wall_secs", Some(n.to_string()))?
                }
            }
            "budget" => {
                if value == "off" {
                    let t = ups(&text, "run", "budget_usd", None)?;
                    ups(&t, "run", "budget_micros", None)?
                } else {
                    let micros: u64 = value
                        .trim_start_matches('$')
                        .parse::<f64>()
                        .map(|d| (d * 1_000_000.0) as u64)
                        .map_err(|_| format!("budget needs dollars (or off), got '{value}'"))?;
                    // always a float literal - a whole-dollar integer
                    // literal is the $100 -> $10 bug (2026-09-12)
                    let t = ups(&text, "run", "budget_usd", Some(usd_literal(micros)))?;
                    ups(&t, "run", "budget_micros", None)?
                }
            }
            "critic-steps" => {
                let n: u64 = value
                    .parse()
                    .map_err(|_| format!("critic-steps needs a number, got '{value}'"))?;
                ups(&text, "critic", "max_steps", Some(n.to_string()))?
            }
            "critic-wall" => {
                let n: u64 = value
                    .parse()
                    .map_err(|_| format!("critic-wall needs seconds, got '{value}'"))?;
                ups(&text, "critic", "wall_secs", Some(n.to_string()))?
            }
            "critic-budget" => {
                let micros: u64 = value
                    .trim_start_matches('$')
                    .parse::<f64>()
                    .map(|d| (d * 1_000_000.0) as u64)
                    .map_err(|_| format!("critic-budget needs dollars, got '{value}'"))?;
                ups(&text, "critic", "budget_micros", Some(micros.to_string()))?
            }
            other => return Err(format!("unknown cap '{other}'")),
        };
        std::fs::write(&self.config_path, out)
            .map_err(|e| format!("write {}: {e}", self.config_path.display()))
    }

    /// The armed session budget (micro-USD): `Some` when the config or
    /// a flag declared one, `None` = uncapped (Eric 2026-09-12).
    #[must_use]
    pub fn budget_micros(&self) -> Option<u64> {
        self.inner.budget_micros()
    }

    /// The counter the session's budget guard binds (ProviderReported
    /// on this user-facing surface).
    #[must_use]
    pub fn budget_guard_mode(&self) -> crate::BudgetGuardMode {
        self.inner.budget_guard_mode()
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
    max_steps: Option<u32>,
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

/// The TUI-launch variant (zero-config stranger path, Eric 2026-09-12):
/// a fresh first-run session opens with an uncredentialed default model.
pub fn load_session_lenient(
    config: &Path,
    log_root: &Path,
    feedback: bool,
    max_steps: Option<u32>,
) -> Result<ReplSession, LoopError> {
    ReplSession::load_lenient(config, log_root, feedback, max_steps)
}

/// One goal, one session, end to end: load the kernel, run the mission,
/// return the result. The `hs-repl run` path.
pub fn run_one_shot(
    config: &Path,
    log_root: &Path,
    goal: &str,
    feedback: bool,
    max_steps: Option<u32>,
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

/// Completions for a command prefix. Sigil-prefixed input completes
/// ("/" canonical, ":" the backward-compatible alias); goal text
/// never does.
#[must_use]
pub fn command_completions(prefix: &str) -> Vec<String> {
    let sigil = if prefix.starts_with('/') {
        '/'
    } else if prefix.starts_with(':') {
        ':'
    } else {
        return Vec::new();
    };
    REPL_COMMANDS
        .iter()
        .filter(|c| c[1..].starts_with(&prefix[1..]))
        .map(|c| format!("{sigil}{}", &c[1..]))
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


/// Eric 2026-09-10: resuming a session whose last mission died AT a
/// cap must say so and name the fix (/caps); a clean outcome stays
/// quiet. `outcome` is the last GoalUpdate's outcome string.
#[must_use]
pub fn capped_resume_notice(outcome: Option<&str>) -> Option<String> {
    let o = outcome?;
    if !(o.contains("exhausted") || o.contains("killed")) {
        return None;
    }
    Some(format!(
        "\u{26a0} last mission ended at the cap ({o}) - raise it with /caps (e.g. /caps steps 100) before re-running the goal"
    ))
}

/// The last mission outcome recorded on a stream (its final
/// GoalUpdate's outcome field) - feeds the resume banner.
#[must_use]
pub fn last_mission_outcome(log_root: &Path, stream_id: uuid::Uuid) -> Option<String> {
    let r = hs_log::StreamReader::open(log_root, stream_id).ok()?;
    let events = r.events().ok()?;
    let e = events
        .iter()
        .rev()
        .find(|e| e.kind == hs_core::EventKind::GoalUpdate)?;
    let bytes = r.resolve_payload(e).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("outcome")?.as_str().map(str::to_owned)
}


// ---------------------------------------------------------------------
// /models add + zero-config stranger path (RED: tui_addprovider_red.rs,
// Eric 2026-09-12). A provider is configuration, so the TUI writes
// configuration: a [[providers]] entry (provmodel shape) plus a
// [[models]] block on the generic plugin, and bare `hairspring`
// auto-creates the rig and the session dir.
// ---------------------------------------------------------------------

fn valid_provider_name(name: &str) -> bool {
    // The env convention HS_<NAME>_API_KEY must stay derivable.
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
}

fn valid_base_url(url: &str) -> bool {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    match rest {
        Some(r) => !r.is_empty() && !r.chars().any(char::is_whitespace) && !r.contains('"'),
        None => false,
    }
}

fn valid_model_id(model: &str) -> bool {
    !model.is_empty() && !model.chars().any(|c| c == '\n' || c == '"')
}

/// Model names declared in a rig TOML (the [[models]] blocks only).
fn rig_model_names(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_models = false;
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            in_models = t == "[[models]]";
        } else if in_models && t.starts_with("name") {
            if let Some(eq) = t.find('=') {
                out.push(t[eq + 1..].trim().trim_matches('"').to_string());
            }
            in_models = false; // one name per block
        }
    }
    out
}

/// The directory the rig's plugin binaries live in, derived from the
/// first [[models]] command path (installed and dev layouts both work).
fn rig_plugin_dir(text: &str) -> Result<std::path::PathBuf, String> {
    let mut in_models = false;
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            in_models = t == "[[models]]";
        } else if in_models && t.starts_with("command") {
            let start = t.find('"').ok_or("[[models]] command has no quoted path")?;
            let end = t[start + 1..]
                .find('"')
                .ok_or("[[models]] command path unterminated")?
                + start
                + 1;
            let bin = std::path::PathBuf::from(&t[start + 1..end]);
            return bin
                .parent()
                .map(std::path::Path::to_path_buf)
                .ok_or_else(|| "[[models]] command path has no parent dir".to_string());
        }
    }
    Err("no [[models]] command found in the rig".to_string())
}

/// The /models add writer: lands the [[providers]] entry (provmodel
/// shape) and the rig's [[models]] block in one validated step - bad
/// input writes nothing.
pub fn add_provider(
    rig: &Path,
    providers_toml: &Path,
    name: &str,
    base_url: &str,
    model: &str,
) -> Result<(), String> {
    let name = name.trim();
    let base_url = base_url.trim();
    let model = model.trim();
    if !valid_provider_name(name) {
        return Err(format!(
            "bad provider name {name:?} - lowercase letters, digits, dashes (e.g. openrouter)"
        ));
    }
    if !valid_base_url(base_url) {
        return Err(format!(
            "bad base URL {base_url:?} - the http(s) chat completions endpoint"
        ));
    }
    if !valid_model_id(model) {
        return Err(format!("bad model id {model:?}"));
    }
    if name == "deepseek" || name == "glm" {
        return Err(format!("{name} is built in already"));
    }
    let rig_text = std::fs::read_to_string(rig)
        .map_err(|e| format!("read {}: {e}", rig.display()))?;
    if rig_model_names(&rig_text).iter().any(|n| n == name) {
        return Err(format!("model {name:?} already exists in the rig"));
    }
    if providers_toml.is_file() {
        let cfgs = crate::realmodel::load_providers_toml(providers_toml)?;
        if cfgs.iter().any(|c| c.name == name) {
            return Err(format!("provider {name:?} already declared"));
        }
    }
    let plugin_dir = rig_plugin_dir(&rig_text)?;
    let provmodel = plugin_dir.join("hs-plugin-provmodel");

    // providers.toml: created with a header when missing, appended after.
    let entry = format!(
        "[[providers]]\nname = \"{name}\"\nbase_url = \"{base_url}\"\nmodel = \"{model}\"\n"
    );
    let new_prov = if providers_toml.is_file() {
        let old = std::fs::read_to_string(providers_toml)
            .map_err(|e| format!("read {}: {e}", providers_toml.display()))?;
        let mut t = old;
        if !t.ends_with('\n') {
            t.push('\n');
        }
        t.push('\n');
        t.push_str(&entry);
        t
    } else {
        format!(
            "# Declared from the TUI (/models add); the generic provmodel plugin serves them.\n{entry}"
        )
    };
    toml::from_str::<toml::Value>(&new_prov)
        .map_err(|e| format!("providers TOML would not parse: {e}"))?;

    // The rig gains one [[models]] block on the generic plugin.
    let mut new_rig = rig_text;
    if !new_rig.ends_with('\n') {
        new_rig.push('\n');
    }
    new_rig.push_str(&format!(
        "\n[[models]]\nname = \"{name}\"\ncommand = [\"{}\", \"{name}\"]\nsubjects = [\"*\"]\n",
        provmodel.display()
    ));
    toml::from_str::<toml::Value>(&new_rig)
        .map_err(|e| format!("rig would not parse: {e}"))?;

    if let Some(parent) = providers_toml.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    std::fs::write(providers_toml, new_prov)
        .map_err(|e| format!("write {}: {e}", providers_toml.display()))?;
    std::fs::write(rig, new_rig).map_err(|e| format!("write {}: {e}", rig.display()))?;
    Ok(())
}

/// What one wizard line produces.
#[derive(Debug)]
pub enum WizardFeed {
    /// The next prompt to print.
    Next(&'static str),
    /// All four fields collected.
    Ready {
        name: String,
        base_url: String,
        model: String,
        key: String,
    },
    Cancelled,
}

#[derive(Debug, PartialEq)]
enum WStep {
    Name,
    BaseUrl,
    Model,
    Key,
}

/// The /models add conversation: name -> base URL -> model id -> key.
/// Pure state; hs-repl drives it over the composer (key step is secret).
#[derive(Debug)]
pub struct AddProviderWizard {
    step: WStep,
    name: String,
    base_url: String,
    model: String,
}

impl Default for AddProviderWizard {
    fn default() -> Self {
        Self::new()
    }
}

impl AddProviderWizard {
    #[must_use]
    pub fn new() -> Self {
        AddProviderWizard {
            step: WStep::Name,
            name: String::new(),
            base_url: String::new(),
            model: String::new(),
        }
    }

    /// The prompt for the step the wizard is on.
    #[must_use]
    pub fn prompt(&self) -> &'static str {
        match self.step {
            WStep::Name => "provider name (lowercase, e.g. openrouter):",
            WStep::BaseUrl => {
                "base URL (the chat completions endpoint, e.g. https://openrouter.ai/api/v1/chat/completions):"
            }
            WStep::Model => "model id (e.g. openai/gpt-5.2):",
            WStep::Key => "API key (input hidden, never written to history):",
        }
    }

    /// The key step hides the composer input.
    #[must_use]
    pub fn is_secret(&self) -> bool {
        self.step == WStep::Key
    }

    pub fn feed(&mut self, line: &str) -> Result<WizardFeed, String> {
        let v = line.trim();
        if v == "/cancel" {
            return Ok(WizardFeed::Cancelled);
        }
        if v.is_empty() {
            return Err("empty - type a value, or /cancel to stop".into());
        }
        match self.step {
            WStep::Name => {
                if !valid_provider_name(v) {
                    return Err(format!(
                        "bad name {v:?} - lowercase letters, digits, dashes"
                    ));
                }
                self.name = v.to_string();
                self.step = WStep::BaseUrl;
                Ok(WizardFeed::Next(self.prompt()))
            }
            WStep::BaseUrl => {
                if !valid_base_url(v) {
                    return Err(format!(
                        "bad base URL {v:?} - starts http:// or https://"
                    ));
                }
                self.base_url = v.to_string();
                self.step = WStep::Model;
                Ok(WizardFeed::Next(self.prompt()))
            }
            WStep::Model => {
                if !valid_model_id(v) {
                    return Err(format!("bad model id {v:?}"));
                }
                self.model = v.to_string();
                self.step = WStep::Key;
                Ok(WizardFeed::Next(self.prompt()))
            }
            WStep::Key => {
                if v.chars().any(char::is_whitespace) {
                    return Err(
                        "the key contains whitespace - paste it exactly as issued".into(),
                    );
                }
                Ok(WizardFeed::Ready {
                    name: self.name.clone(),
                    base_url: self.base_url.clone(),
                    model: self.model.clone(),
                    key: v.to_string(),
                })
            }
        }
    }
}

/// Where a bare `hairspring` keeps its run state:
/// `$XDG_DATA_HOME/hairspring/run`, else `~/.local/share/hairspring/run`.
#[must_use]
pub fn default_session_dir() -> std::path::PathBuf {
    if let Ok(x) = std::env::var("XDG_DATA_HOME") {
        if !x.is_empty() {
            return std::path::PathBuf::from(x)
                .join("hairspring")
                .join("run");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    std::path::PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("hairspring")
        .join("run")
}

/// `--dir` when given; the automatic session dir (created) otherwise.
pub fn resolve_session_dir(given: Option<&str>) -> Result<std::path::PathBuf, String> {
    let p = given.map_or_else(default_session_dir, std::path::PathBuf::from);
    std::fs::create_dir_all(&p).map_err(|e| format!("create {}: {e}", p.display()))?;
    Ok(p)
}

/// `--config` when given; the rig in the config dir otherwise, written
/// from the shipped template on first run (`setup::ensure_config`).
pub fn resolve_config_path(given: Option<&str>) -> Result<std::path::PathBuf, String> {
    if let Some(g) = given {
        return Ok(std::path::PathBuf::from(g));
    }
    crate::setup::ensure_config()
}
