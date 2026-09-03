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
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Instant, SystemTime};

#[derive(Debug)]
pub enum KernelError {
    Config(String),
    Protocol(String),
    UnknownTool(String),
    UnknownModel(String),
    Gated { name: String, subject: String },
    Plugin(String),
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
    pub completion: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd_micros: i64,
    pub latency_ms: u32,
    pub model: String,
}

struct PluginProc {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl PluginProc {
    fn spawn(command: &[String]) -> Result<Self, KernelError> {
        let (prog, args) = command
            .split_first()
            .ok_or_else(|| KernelError::Config("empty command".into()))?;
        let mut child = Command::new(prog)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| KernelError::Plugin(format!("spawn {}: {e}", command.join(" "))))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Ok(PluginProc {
            child,
            stdin,
            stdout,
            next_id: 0,
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
        let mut line = String::new();
        let n = self
            .stdout
            .read_line(&mut line)
            .map_err(|e| KernelError::Plugin(format!("read: {e}")))?;
        if n == 0 {
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
            return Err(KernelError::Plugin(format!("plugin error: {err}")));
        }
        Ok(v["result"].clone())
    }
}

struct PluginSlot {
    entry: PluginEntry,
    proc: Option<PluginProc>,
}

impl PluginSlot {
    fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, KernelError> {
        let mut p = self
            .proc
            .take()
            .ok_or_else(|| KernelError::Plugin("not spawned".into()))?;
        match p.call(method, params.clone()) {
            Ok(r) => {
                self.proc = Some(p);
                Ok(r)
            }
            Err(e) => {
                let _ = p.child.kill();
                let _ = p.child.wait();
                // one restart, then give up
                let mut fresh = PluginProc::spawn(&self.entry.command)?;
                let r = fresh.call(method, params).map_err(|_| e)?;
                self.proc = Some(fresh);
                Ok(r)
            }
        }
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
        let spawn_describe =
            |entry: &PluginEntry, kind: &'static str| -> Result<PluginSlot, KernelError> {
                let mut p = PluginProc::spawn(&entry.command)?;
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
        let t0 = Instant::now();
        let result = {
            let mut models = self.models.borrow_mut();
            models
                .get_mut(&name)
                .unwrap()
                .call("model.call", serde_json::json!({"prompt": prompt}))
        };
        let latency_ms = t0.elapsed().as_millis() as u32;
        let r = result?;
        let out = ModelOutcome {
            completion: r["completion"].as_str().unwrap_or("").to_string(),
            input_tokens: r["input_tokens"].as_u64().unwrap_or(0),
            output_tokens: r["output_tokens"].as_u64().unwrap_or(0),
            cost_usd_micros: r["cost_usd_micros"].as_i64().unwrap_or(0),
            latency_ms,
            model: name.clone(),
        };
        self.record(
            EventKind::ModelCall,
            serde_json::json!({
                "model": name, "prompt": prompt, "completion": out.completion,
                "input_tokens": out.input_tokens, "output_tokens": out.output_tokens,
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
