//! HAIRSPRING gate 5 - sub-agent spawner + swarm operators (spec 10 row 5,
//! the gate5 design). A delegated subtask runs the SAME
//! substrate as a child stream: same log root, same kernel config, one
//! Spawn event on the parent stream linking to the child `stream_id`.
//! Delegation overhead is measured in milliseconds, not deployment.

use hs_core::{EventBuilder, EventKind, Payload};
use hs_log::StreamWriter;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Handle to a spawned child run.
pub struct Child {
    pub stream_id: uuid::Uuid,
    pub mission: String,
    /// Optional delegation-time model override (must be registered in
    /// the kernel config); None = the config's default model.
    pub model: Option<String>,
    /// This child's delegation depth (parent's + 1), set at spawn.
    pub depth: u32,
    pub work_dir: PathBuf,
}

/// What the parent collects when a child finishes.
#[derive(Debug, Clone)]
pub struct ChildReport {
    pub stream_id: uuid::Uuid,
    pub mission: String,
    /// The child's default model, from its own kernel config - the
    /// spawner reports it so the UI never invents delegation context.
    pub model: String,
    pub passed: bool,
    pub steps: u32,
    pub cost_usd_micros: i64,
}

#[derive(Debug)]
pub enum SpawnError {
    Log(hs_log::LogError),
    Kernel(hs_kernel::KernelError),
    Loop(hs_loop::LoopError),
    Config(String),
}

impl From<hs_log::LogError> for SpawnError {
    fn from(e: hs_log::LogError) -> Self {
        Self::Log(e)
    }
}
impl From<hs_kernel::KernelError> for SpawnError {
    fn from(e: hs_kernel::KernelError) -> Self {
        Self::Kernel(e)
    }
}
impl From<hs_loop::LoopError> for SpawnError {
    fn from(e: hs_loop::LoopError) -> Self {
        Self::Loop(e)
    }
}

/// The spawner: creates child streams in the parent's log root.
pub struct Spawner {
    log_root: PathBuf,
    kernel_config: PathBuf,
    feedback: bool,
    max_steps: u32,
}

impl Spawner {
    #[must_use]
    pub fn new(log_root: &Path, kernel_config: &Path, feedback: bool, max_steps: u32) -> Self {
        Self {
            log_root: log_root.to_path_buf(),
            kernel_config: kernel_config.to_path_buf(),
            feedback,
            max_steps,
        }
    }

    /// Spawn a child: create its stream in the same log root with a
    /// mission-start event, then append a Spawn event naming the child
    /// `stream_id` to the parent stream. Returns the child handle plus the
    /// delegation overhead in milliseconds (decision -> linked child stream).
    pub fn spawn(
        &self,
        parent_log: &Path,
        parent_stream: uuid::Uuid,
        mission: &str,
    ) -> Result<(Child, f64), SpawnError> {
        let t0 = Instant::now();
        let child_id = uuid::Uuid::new_v4();

        // child stream + first event: it exists on the substrate from this
        // moment, verifiable like any other stream
        let mut cw = StreamWriter::create(&self.log_root, child_id)?;
        cw.append(
            EventBuilder::new(EventKind::GoalUpdate).payload(Payload::Inline(
                serde_json::to_vec(&serde_json::json!({
                    "mission": mission, "child_of": parent_stream, "done": false,
                }))
                .expect("json! values serialize"),
            )),
        )?;
        drop(cw);

        // parent records the delegation
        let mut pw = StreamWriter::resume(parent_log, parent_stream)?.writer;
        pw.append(
            EventBuilder::new(EventKind::Spawn).payload(Payload::Inline(
                serde_json::to_vec(&serde_json::json!({
                    "child_stream_id": child_id,
                    "mission": mission,
                    "budget": {"max_steps": self.max_steps},
                }))
                .expect("json! values serialize"),
            )),
        )?;

        let overhead_ms = t0.elapsed().as_secs_f64() * 1000.0;
        Ok((
            Child {
                stream_id: child_id,
                mission: mission.to_string(),
                model: None,
                depth: 0,
                work_dir: self.log_root.join("work").join(mission),
            },
            overhead_ms,
        ))
    }

    /// Drive a child to completion on this thread (parallel = N threads,
    /// one Spawner clone per thread via `new`).
    /// Create the child stream WITHOUT appending to the parent
    /// stream: for in-process callers (the agent.spawn tool path),
    /// where the parent's own loop writer owns that stream and a
    /// second appender would corrupt seq numbering. The loop books the
    /// Spawn link itself after the call returns.
    pub fn spawn_child(
        &self,
        parent_stream: uuid::Uuid,
        child_id: uuid::Uuid,
        mission: &str,
        model: Option<&str>,
        depth: u32,
    ) -> Result<(Child, f64), SpawnError> {
        let t0 = Instant::now();
        let mut cw = StreamWriter::create(&self.log_root, child_id)?;
        cw.append(
            EventBuilder::new(EventKind::GoalUpdate).payload(Payload::Inline(
                serde_json::to_vec(&serde_json::json!({
                    "mission": mission, "child_of": parent_stream, "done": false,
                }))
                .expect("json! values serialize"),
            )),
        )?;
        drop(cw);
        let overhead_ms = t0.elapsed().as_secs_f64() * 1000.0;
        Ok((
            Child {
                stream_id: child_id,
                mission: mission.to_string(),
                model: model.map(std::string::ToString::to_string),
                depth,
                work_dir: self.log_root.join("work").join(mission),
            },
            overhead_ms,
        ))
    }

    pub fn run_to_completion(&self, child: &Child) -> Result<ChildReport, SpawnError> {
        let kernel = hs_kernel::Kernel::load_with_log(&self.kernel_config, &self.log_root)?;
        let model = match &child.model {
            Some(name) => {
                if !kernel.has_model(name) {
                    return Err(SpawnError::Config(format!(
                        "agent.spawn: model \"{name}\" is not registered in the kernel config"
                    )));
                }
                name.clone()
            }
            None => kernel
                .model_names()
                .into_iter()
                .find(|(_, is_default)| *is_default).map_or_else(|| "(unknown)".to_string(), |(name, _)| name),
        };
        let mut l = hs_loop::InnerLoop::with_stream(
            kernel,
            &self.log_root,
            child.stream_id,
            self.feedback,
            self.max_steps,
        )?;
        // A child is a full operator: same native tool set as the
        // interactive surface, including delegation (the depth guard
        // in hs-plugin-swarm bounds the tree).
        let mut tools = hs_loop::toolschema::tb_tools();
        tools.push(hs_loop::toolschema::agent_spawn_tool());
        l.set_tools(serde_json::Value::Array(tools));
        l.set_model_override(child.model.clone())?;
        // The child loop knows its own depth explicitly - env is
        // process-global and this loop shares the parent's plugin
        // process with concurrent sibling threads.
        l.set_swarm_depth(child.depth);
        let r = l.run_mission(&child.mission)?;
        Ok(ChildReport {
            stream_id: child.stream_id,
            mission: child.mission.clone(),
            model,
            passed: r.passed,
            steps: r.steps,
            cost_usd_micros: l.total_cost_micros() as i64,
        })
    }
}
