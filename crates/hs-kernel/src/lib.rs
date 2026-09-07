//! HAIRSPRING gate 2 - plugin kernel + Rails (spec section 10 row 2).
//!
//! Everything is a plugin (DSH): tools, models, rails are executables
//! declared in one TOML config, speaking newline-delimited JSON over stdio.
//! Adding a capability is a config change; the kernel hot-reloads on config
//! mtime, so the harness process is never restarted ("zero redeploy").
//! Rails (openJiuwen): rho = (hooks, handler, priority), dispatched in
//! priority order with name tie-break, visibility-gated per subject.

use hs_core::{EventBuilder, EventKind, Payload};
use hs_log::{LogError, StreamWriter};
use serde::Deserialize;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Instant, SystemTime};

#[derive(Debug)]
pub enum KernelError {
    Config(String),
    Protocol(String),
    UnknownTool(String),
    UnknownModel(String),
    Gated { name: String, subject: String },
    Plugin(String),
    /// Application-level error from a HEALTHY plugin process (a well-formed
    /// {"error": ...} response): usage mistakes, bad args, provider errors
    /// after the plugin's own retries. NOT a supervisor strike - the process
    /// stays up and keeps serving (Eric 2026-09-05: exploration is never
    /// punished; ab2/17123 died when a usage error counted as strike 3).
    PluginApp { name: String, detail: String },
    /// Supervisor terminal state: the plugin failed `strikes` consecutive
    /// call attempts (crash, spawn failure, or lease expiry). `detail` is
    /// the most recent REAL failure - never a stale earlier error.
    PluginDead {
        name: String,
        strikes: u32,
        detail: String,
    },
    Log(LogError),
}
impl From<LogError> for KernelError {
    fn from(e: LogError) -> Self {
        KernelError::Log(e)
    }
}
impl std::fmt::Display for KernelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(e) => write!(f, "config: {e}"),
            Self::Protocol(e) => write!(f, "protocol: {e}"),
            Self::UnknownTool(n) => write!(f, "unknown tool: {n}"),
            Self::UnknownModel(n) => write!(f, "unknown model: {n}"),
            Self::Gated { name, subject } => write!(f, "{name} not visible to subject {subject}"),
            Self::Plugin(e) => write!(f, "plugin: {e}"),
            Self::PluginApp { name, detail } => write!(f, "plugin {name} app error: {detail}"),
            Self::PluginDead {
                name,
                strikes,
                detail,
            } => write!(f, "plugin {name} dead after {strikes} strikes: {detail}"),
            Self::Log(e) => write!(f, "log: {e}"),
        }
    }
}
impl std::error::Error for KernelError {}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginEntry {
    pub name: String,
    pub command: Vec<String>,
    #[serde(default = "all_subjects")]
    pub subjects: Vec<String>,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub hooks: Vec<String>,
    #[serde(default)]
    pub priority: i64,
    /// Per-call lease in seconds: a plugin that does not answer within the
    /// lease is killed and the attempt counts as a strike. Default 1800s
    /// (model calls legitimately run to ~1500s under provider timeouts).
    #[serde(default)]
    pub lease_secs: Option<u64>,
}
fn all_subjects() -> Vec<String> {
    vec!["*".into()]
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ConfigFile {
    #[serde(default)]
    tools: Vec<PluginEntry>,
    #[serde(default)]
    models: Vec<PluginEntry>,
    #[serde(default)]
    rails: Vec<PluginEntry>,
}

#[derive(Debug)]
pub struct ToolCallOutcome {
    pub output: serde_json::Value,
    pub latency_ms: u32,
}
#[derive(Debug)]
pub struct ModelOutcome {
    /// Provider-reported prompt-cache hits (0 when the provider/fixture
    /// does not report any); observability for the KV-cache design.
    pub completion: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    /// The reasoning text itself (empty when the provider returns none).
    pub reasoning_content: String,
    pub cached_tokens: u64,
    pub cost_usd_micros: i64,
    pub latency_ms: u32,
    pub model: String,
}

/// Default per-call lease when the config does not set one. Model calls
/// legitimately run to ~1500s under provider-side timeouts, so the default
/// is generous; tools should set a tighter lease in config.
const DEFAULT_LEASE_SECS: u64 = 1800;

struct PluginProc {
    child: Child,
    stdin: ChildStdin,
    /// Lines produced by the plugin, pumped by a dedicated reader thread so
    /// a hung plugin is detectable with a recv deadline (the lease).
    lines: std::sync::mpsc::Receiver<Result<String, String>>,
    next_id: u64,
    lease: std::time::Duration,
}

impl PluginProc {
    /// spawn + optional stderr capture: a wedged or dying plugin must leave
    /// its stderr somewhere an operator can read (conan-17302, 2026-09-07:
    /// 19 min of silence with stderr wired to /dev/null). Falls back to
    /// Stdio::null when no log path is configured - never pipe: an
    /// undrained pipe is itself a wedge vector.
    fn spawn(
        command: &[String],
        lease_secs: Option<u64>,
        stderr_log: Option<&std::path::Path>,
    ) -> Result<Self, KernelError> {
        let (prog, args) = command
            .split_first()
            .ok_or_else(|| KernelError::Config("empty command".into()))?;
        let mut child = Command::new(prog)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(match stderr_log {
                Some(p) => {
                    if let Some(parent) = p.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(p)
                        .map(Stdio::from)
                        .unwrap_or(Stdio::null())
                }
                None => Stdio::null(),
            })
            .spawn()
            .map_err(|e| KernelError::Plugin(format!("spawn {}: {e}", command.join(" "))))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();
        std::thread::spawn(move || {
            let mut stdout = stdout;
            loop {
                let mut line = String::new();
                match stdout.read_line(&mut line) {
                    Ok(0) => {
                        let _ = tx.send(Ok(String::new())); // EOF
                        return;
                    }
                    Ok(_) => {
                        if tx.send(Ok(line)).is_err() {
                            return; // supervisor gone
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(format!("read: {e}")));
                        return;
                    }
                }
            }
        });
        Ok(PluginProc {
            child,
            stdin,
            lines: rx,
            next_id: 0,
            lease: std::time::Duration::from_secs(lease_secs.unwrap_or(DEFAULT_LEASE_SECS)),
        })
    }

    /// One request/response round trip. A dead plugin (EOF / no response /
    /// write failure) is reported as PluginError so the caller can respawn
    /// and retry exactly once.
    fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, KernelError> {
        self.next_id += 1;
        let id = self.next_id;
        let req = serde_json::json!({"id": id, "method": method, "params": params});
        writeln!(self.stdin, "{}", req)
            .and_then(|_| self.stdin.flush())
            .map_err(|e| KernelError::Plugin(format!("write: {e}")))?;
        let line = match self.lines.recv_timeout(self.lease) {
            Ok(Ok(l)) => l,
            Ok(Err(e)) => return Err(KernelError::Plugin(e)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                return Err(KernelError::Plugin(format!(
                    "lease expired after {}s (plugin hung)",
                    self.lease.as_secs()
                )));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(KernelError::Plugin("reader thread gone".into()));
            }
        };
        if line.is_empty() {
            return Err(KernelError::Plugin("plugin exited (EOF)".into()));
        }
        let v: serde_json::Value = serde_json::from_str(&line)
            .map_err(|e| KernelError::Protocol(format!("bad json from plugin: {e}")))?;
        if v["id"] != id {
            return Err(KernelError::Protocol(format!(
                "id mismatch: sent {id}, got {}",
                v["id"]
            )));
        }
        if let Some(err) = v.get("error") {
            // a well-formed error response from a LIVE process: the caller
            // (PluginSlot) must not strike or kill for this
            return Err(KernelError::PluginApp {
                name: String::new(),
                detail: err.to_string(),
            });
        }
        Ok(v["result"].clone())
    }
}

/// Consecutive call-attempt failures before the slot is declared dead.
/// Crash, spawn failure, and lease expiry all count as strikes.
const MAX_STRIKES: u32 = 3;

struct PluginSlot {
    entry: PluginEntry,
    proc: Option<PluginProc>,
    strikes: u32,
    stderr_dir: Option<std::path::PathBuf>,
}

impl PluginSlot {
    fn spawn_fresh(&self) -> Result<PluginProc, KernelError> {
        let log = self.stderr_dir.as_ref().map(|d| {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|t| t.as_nanos())
                .unwrap_or(0);
            d.join(format!("{}-{}.stderr.log", self.entry.name, nanos))
        });
        PluginProc::spawn(&self.entry.command, self.entry.lease_secs, log.as_deref())
    }

    /// Supervisor contract (phase 1, design D4): every attempt starts by
    /// ensuring a live process - a None slot respawns from config, so a
    /// previously dead slot recovers the moment the plugin can spawn again.
    /// Bounded retries: after MAX_STRIKES consecutive failures the call
    /// returns PluginDead naming the plugin and the most recent real cause.
    /// Any success resets the strike counter.
    fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, KernelError> {
        let mut detail = String::new();
        for _ in 0..MAX_STRIKES {
            if self.proc.is_none() {
                match self.spawn_fresh() {
                    Ok(p) => self.proc = Some(p),
                    Err(e) => {
                        self.strikes += 1;
                        detail = e.to_string();
                        continue;
                    }
                }
            }
            let mut p = self.proc.take().expect("proc ensured above");
            match p.call(method, params.clone()) {
                Ok(r) => {
                    self.proc = Some(p);
                    self.strikes = 0;
                    return Ok(r);
                }
                Err(KernelError::PluginApp { name: _, detail }) => {
                    // application-level error from a live process: hand the
                    // error to the caller, keep the process, no strike
                    self.proc = Some(p);
                    return Err(KernelError::PluginApp {
                        name: self.entry.name.clone(),
                        detail,
                    });
                }
                Err(e) => {
                    let _ = p.child.kill();
                    let _ = p.child.wait();
                    self.strikes += 1;
                    detail = e.to_string();
                }
            }
        }
        Err(KernelError::PluginDead {
            name: self.entry.name.clone(),
            strikes: self.strikes,
            detail,
        })
    }
}

pub struct Kernel {
    config_path: PathBuf,
    config_mtime: SystemTime,
    tools: RefCell<HashMap<String, PluginSlot>>,
    models: RefCell<HashMap<String, PluginSlot>>,
    rails: RefCell<Vec<PluginSlot>>,
    log: RefCell<Option<StreamWriter>>,
    log_root: Option<PathBuf>,
    stream_id: RefCell<Option<uuid::Uuid>>,
}

impl Kernel {
    pub fn load(config: &Path) -> Result<Self, KernelError> {
        Self::load_inner(config, None)
    }

    /// True when this kernel was built with a log root (load_with_log):
    /// dispatch records and plugin stderr capture are active only then.
    pub fn has_log_root(&self) -> bool {
        self.log_root.is_some()
    }

    /// Load and record every tool/model call to a fresh stream in `log_root`.
    pub fn load_with_log(config: &Path, log_root: &Path) -> Result<Self, KernelError> {
        Self::load_inner(config, Some(log_root))
    }

    fn load_inner(config: &Path, log_root: Option<&Path>) -> Result<Self, KernelError> {
        let (parsed, mtime) = read_config(config)?;
        let k = Kernel {
            config_path: config.to_path_buf(),
            config_mtime: mtime,
            tools: RefCell::new(HashMap::new()),
            models: RefCell::new(HashMap::new()),
            rails: RefCell::new(vec![]),
            log: RefCell::new(None),
            log_root: log_root.map(|p| p.to_path_buf()),
            stream_id: RefCell::new(None),
        };
        k.apply_config(parsed)?;
        Ok(k)
    }

    fn apply_config(&self, parsed: ConfigFile) -> Result<(), KernelError> {
        let stderr_dir = self.log_root.as_ref().map(|r| r.join("stderr"));
        let spawn_describe =
            |entry: &PluginEntry, kind: &'static str| -> Result<PluginSlot, KernelError> {
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|t| t.as_nanos())
                    .unwrap_or(0);
                let log = stderr_dir
                    .as_ref()
                    .map(|d| d.join(format!("{}-{}.stderr.log", entry.name, nanos)));
                let mut p = PluginProc::spawn(&entry.command, entry.lease_secs, log.as_deref())?;
                let desc = p.call("describe", serde_json::json!({}))?;
                if desc["name"].as_str() != Some(entry.name.as_str()) {
                    return Err(KernelError::Protocol(format!(
                        "plugin {} describes itself as {}",
                        entry.name, desc["name"]
                    )));
                }
                if desc["kind"].as_str() != Some(kind) {
                    return Err(KernelError::Protocol(format!(
                        "plugin {} claims kind {}, expected {kind}",
                        entry.name, desc["kind"]
                    )));
                }
                Ok(PluginSlot {
                    entry: entry.clone(),
                    proc: Some(p),
                    strikes: 0,
                    stderr_dir: stderr_dir.clone(),
                })
            };
        let mut tools = self.tools.borrow_mut();
        for e in &parsed.tools {
            if !tools.contains_key(&e.name) {
                tools.insert(e.name.clone(), spawn_describe(e, "tool")?);
            }
        }
        tools.retain(|name, _| parsed.tools.iter().any(|e| &e.name == name));
        let mut models = self.models.borrow_mut();
        for e in &parsed.models {
            if !models.contains_key(&e.name) {
                models.insert(e.name.clone(), spawn_describe(e, "model")?);
            }
        }
        models.retain(|name, _| parsed.models.iter().any(|e| &e.name == name));
        // rails: replace wholesale (ordering/gating metadata may have changed)
        let mut rails = self.rails.borrow_mut();
        let mut new_rails = vec![];
        for e in &parsed.rails {
            new_rails.push(spawn_describe(e, "rail")?);
        }
        *rails = new_rails;
        rails.sort_by(|a, b| {
            a.entry
                .priority
                .cmp(&b.entry.priority)
                .then(a.entry.name.cmp(&b.entry.name))
        });
        Ok(())
    }

    /// Hot reload: pick up config changes without restarting the harness.
    pub fn reload_if_changed(&mut self) -> Result<bool, KernelError> {
        let mtime = std::fs::metadata(&self.config_path)
            .and_then(|m| m.modified())
            .map_err(|e| KernelError::Config(e.to_string()))?;
        if mtime <= self.config_mtime {
            return Ok(false);
        }
        let (parsed, mtime) = read_config(&self.config_path)?;
        self.apply_config(parsed)?;
        self.config_mtime = mtime;
        Ok(true)
    }

    fn visible(entry: &PluginEntry, subject: &str) -> bool {
        entry.subjects.iter().any(|s| s == "*" || s == subject)
    }

    pub fn list_tools(&self, subject: &str) -> Vec<PluginEntry> {
        self.tools
            .borrow()
            .values()
            .filter(|s| Self::visible(&s.entry, subject))
            .map(|s| s.entry.clone())
            .collect()
    }
    pub fn list_models(&self) -> Vec<PluginEntry> {
        self.models
            .borrow()
            .values()
            .map(|s| s.entry.clone())
            .collect()
    }

    pub fn call_tool(
        &self,
        subject: &str,
        name: &str,
        args: serde_json::Value,
    ) -> Result<ToolCallOutcome, KernelError> {
        let mut tools = self.tools.borrow_mut();
        let slot = tools
            .get_mut(name)
            .ok_or_else(|| KernelError::UnknownTool(name.into()))?;
        if !Self::visible(&slot.entry, subject) {
            return Err(KernelError::Gated {
                name: name.into(),
                subject: subject.into(),
            });
        }
        drop(tools);
        self.fire_rails("call.pre_tool", subject, name, &args);
        // Dispatch-side record: the stream must name the in-flight plugin
        // BEFORE the call is awaited, so a wedged call is diagnosable while
        // it is wedged (conan-17302 read as total silence for 19 min).
        self.record(
            EventKind::Observation,
            serde_json::json!({
                "plugin": name, "stage": "dispatch", "method": "tool.call", "args": args.clone(),
            }),
            0,
            0,
        )?;
        let t0 = Instant::now();
        let result = {
            let mut tools = self.tools.borrow_mut();
            tools
                .get_mut(name)
                .unwrap()
                .call("tool.call", serde_json::json!({"args": args.clone()}))
        };
        let latency_ms = t0.elapsed().as_millis() as u32;
        match result {
            Ok(r) => {
                self.record(
                    EventKind::ToolCall,
                    serde_json::json!({
                        "plugin": name, "args": args, "result": r,
                    }),
                    latency_ms,
                    0,
                )?;
                self.fire_rails("call.post_tool", subject, name, &r);
                Ok(ToolCallOutcome {
                    output: r,
                    latency_ms,
                })
            }
            Err(e) => {
                self.record(
                    EventKind::Observation,
                    serde_json::json!({
                        "plugin": name, "error": e.to_string(), "stage": "tool.call",
                    }),
                    latency_ms,
                    0,
                )?;
                Err(e)
            }
        }
    }

    pub fn call_model(
        &self,
        subject: &str,
        model: Option<&str>,
        prompt: &str,
    ) -> Result<ModelOutcome, KernelError> {
        self.call_model_with(subject, model, prompt, None)
    }

    /// call_model + native tool schemas: tools is passed to the model
    /// plugin and on to the provider's tools parameter (native tool
    /// calling). None = no tools param (distill, verifier, fixtures).
    pub fn call_model_with(
        &self,
        subject: &str,
        model: Option<&str>,
        prompt: &str,
        tools: Option<&serde_json::Value>,
    ) -> Result<ModelOutcome, KernelError> {
        let name = {
            let models = self.models.borrow();
            match model {
                Some(m) => m.to_string(),
                None => models
                    .values()
                    .find(|s| s.entry.default)
                    .map(|s| s.entry.name.clone())
                    .ok_or_else(|| KernelError::UnknownModel("(no default)".into()))?,
            }
        };
        {
            let models = self.models.borrow();
            let slot = models
                .get(&name)
                .ok_or_else(|| KernelError::UnknownModel(name.clone()))?;
            if !Self::visible(&slot.entry, subject) {
                return Err(KernelError::Gated {
                    name,
                    subject: subject.into(),
                });
            }
        }
        self.fire_rails(
            "call.pre_model",
            subject,
            &name,
            &serde_json::json!({"prompt": prompt}),
        );
        self.record(
            EventKind::Observation,
            serde_json::json!({
                "plugin": name, "stage": "dispatch", "method": "model.call", "prompt": prompt,
            }),
            0,
            0,
        )?;
        let t0 = Instant::now();
        let result = {
            let mut models = self.models.borrow_mut();
            models
                .get_mut(&name)
                .unwrap()
                .call(
                    "model.call",
                    match tools {
                        Some(t) => serde_json::json!({"prompt": prompt, "tools": t}),
                        None => serde_json::json!({"prompt": prompt}),
                    },
                )
        };
        let latency_ms = t0.elapsed().as_millis() as u32;
        let r = result?;
        let out = ModelOutcome {
            completion: r["completion"].as_str().unwrap_or("").to_string(),
            input_tokens: r["input_tokens"].as_u64().unwrap_or(0),
            output_tokens: r["output_tokens"].as_u64().unwrap_or(0),
            reasoning_tokens: r["reasoning_tokens"].as_u64().unwrap_or(0),
            reasoning_content: r["reasoning_content"].as_str().unwrap_or("").to_string(),
            cached_tokens: r["cached_tokens"].as_u64().unwrap_or(0),
            cost_usd_micros: r["cost_usd_micros"].as_i64().unwrap_or(0),
            latency_ms,
            model: name.clone(),
        };
        self.record(
            EventKind::ModelCall,
            serde_json::json!({
                "model": name, "prompt": prompt, "completion": out.completion,
                "input_tokens": out.input_tokens, "output_tokens": out.output_tokens,
                "reasoning_tokens": out.reasoning_tokens,
                "reasoning_content": out.reasoning_content,
                "cached_tokens": out.cached_tokens,
            }),
            latency_ms,
            out.cost_usd_micros,
        )?;
        self.fire_rails(
            "call.post_model",
            subject,
            &name,
            &serde_json::json!({"completion": out.completion}),
        );
        Ok(out)
    }

    pub fn call_model_messages(
        &self,
        subject: &str,
        model: Option<&str>,
        messages: &serde_json::Value,
        tools: Option<&serde_json::Value>,
    ) -> Result<ModelOutcome, KernelError> {
        let name = {
            let models = self.models.borrow();
            match model {
                Some(m) => m.to_string(),
                None => models
                    .values()
                    .find(|s| s.entry.default)
                    .map(|s| s.entry.name.clone())
                    .ok_or_else(|| KernelError::UnknownModel("(no default)".into()))?,
            }
        };
        {
            let models = self.models.borrow();
            let slot = models
                .get(&name)
                .ok_or_else(|| KernelError::UnknownModel(name.clone()))?;
            if !Self::visible(&slot.entry, subject) {
                return Err(KernelError::Gated {
                    name,
                    subject: subject.into(),
                });
            }
        }
        self.fire_rails(
            "call.pre_model",
            subject,
            &name,
            &serde_json::json!({"messages": messages}),
        );
        self.record(
            EventKind::Observation,
            serde_json::json!({
                "plugin": name, "stage": "dispatch", "method": "model.call", "messages": messages,
            }),
            0,
            0,
        )?;
        let t0 = Instant::now();
        let result = {
            let mut models = self.models.borrow_mut();
            models
                .get_mut(&name)
                .unwrap()
                .call(
                    "model.call",
                    match tools {
                        Some(t) => serde_json::json!({"messages": messages, "tools": t}),
                        None => serde_json::json!({"messages": messages}),
                    },
                )
        };
        let latency_ms = t0.elapsed().as_millis() as u32;
        let r = result?;
        let out = ModelOutcome {
            completion: r["completion"].as_str().unwrap_or("").to_string(),
            input_tokens: r["input_tokens"].as_u64().unwrap_or(0),
            output_tokens: r["output_tokens"].as_u64().unwrap_or(0),
            reasoning_tokens: r["reasoning_tokens"].as_u64().unwrap_or(0),
            reasoning_content: r["reasoning_content"].as_str().unwrap_or("").to_string(),
            cached_tokens: r["cached_tokens"].as_u64().unwrap_or(0),
            cost_usd_micros: r["cost_usd_micros"].as_i64().unwrap_or(0),
            latency_ms,
            model: name.clone(),
        };
        self.record(
            EventKind::ModelCall,
            serde_json::json!({
                "model": name, "messages": messages, "completion": out.completion,
                "input_tokens": out.input_tokens, "output_tokens": out.output_tokens,
                "reasoning_tokens": out.reasoning_tokens,
                "reasoning_content": out.reasoning_content,
                "cached_tokens": out.cached_tokens,
            }),
            latency_ms,
            out.cost_usd_micros,
        )?;
        self.fire_rails(
            "call.post_model",
            subject,
            &name,
            &serde_json::json!({"completion": out.completion}),
        );
        Ok(out)
    }

    /// Dispatch a lifecycle hook to attached rails: priority order, name
    /// tie-break (openJiuwen eq. 2). Rail failures are contained and logged.
    fn fire_rails(&self, hook: &str, subject: &str, target: &str, payload: &serde_json::Value) {
        let mut rails = self.rails.borrow_mut();
        for slot in rails.iter_mut() {
            if !slot.entry.hooks.iter().any(|h| h == hook) {
                continue;
            }
            if !Self::visible(&slot.entry, subject) {
                continue;
            }
            let r = slot.call(
                "rail.hook",
                serde_json::json!({
                    "hook": hook, "subject": subject, "target": target, "event": payload,
                }),
            );
            if let Err(e) = r {
                let name = slot.entry.name.clone();
                let _ = self.record(
                    EventKind::Observation,
                    serde_json::json!({
                        "rail": name, "hook": hook, "error": e.to_string(), "stage": "rail.hook",
                    }),
                    0,
                    0,
                );
            }
        }
    }

    fn record(
        &self,
        kind: EventKind,
        body: serde_json::Value,
        latency_ms: u32,
        cost: i64,
    ) -> Result<(), KernelError> {
        if self.log_root.is_none() {
            return Ok(());
        }
        let mut log = self.log.borrow_mut();
        if log.is_none() {
            let sid = uuid::Uuid::new_v4();
            *log = Some(StreamWriter::create(self.log_root.as_ref().unwrap(), sid)?);
            *self.stream_id.borrow_mut() = Some(sid);
        }
        let w = log.as_mut().unwrap();
        w.append(
            EventBuilder::new(kind)
                .payload(Payload::Inline(serde_json::to_vec(&body).unwrap()))
                .latency_ms(latency_ms)
                .cost_usd_micros(cost),
        )?;
        Ok(())
    }

    pub fn stream_id(&self) -> Option<uuid::Uuid> {
        *self.stream_id.borrow()
    }
}

fn read_config(path: &Path) -> Result<(ConfigFile, SystemTime), KernelError> {
    let text = std::fs::read_to_string(path).map_err(|e| KernelError::Config(e.to_string()))?;
    let parsed: ConfigFile =
        toml::from_str(&text).map_err(|e| KernelError::Config(e.to_string()))?;
    let mtime = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map_err(|e| KernelError::Config(e.to_string()))?;
    Ok((parsed, mtime))
}

/// Test support.
pub mod testing {
    use hs_core::Event;
    use hs_log::StreamReader;
    use std::path::Path;

    /// Read the single stream in a kernel log dir.
    pub fn read_only_stream(log_root: &Path) -> Vec<Event> {
        let mut entries: Vec<_> = std::fs::read_dir(log_root.join("streams"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(entries.len(), 1);
        let sid = uuid::Uuid::parse_str(&entries.pop().unwrap()).unwrap();
        StreamReader::open(log_root, sid).unwrap().events().unwrap()
    }
}
pub use testing::read_only_stream;

impl std::fmt::Debug for Kernel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Kernel")
            .field("config_path", &self.config_path)
            .finish()
    }
}
