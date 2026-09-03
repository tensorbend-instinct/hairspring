//! HAIRSPRING gate 5 - sub-agent spawner + swarm operators (spec 10 row 5,
//! design docs/gate5-design.md). A delegated subtask runs the SAME
//! substrate as a child stream: same log root, same kernel config, one
//! Spawn event on the parent stream linking to the child stream_id.
//! Delegation overhead is measured in milliseconds, not deployment.

use std::path::{Path, PathBuf};

/// Handle to a spawned child run.
pub struct Child {
    pub stream_id: uuid::Uuid,
    pub mission: String,
    pub work_dir: PathBuf,
}

/// What the parent collects when a child finishes.
#[derive(Debug, Clone)]
pub struct ChildReport {
    pub stream_id: uuid::Uuid,
    pub mission: String,
    pub passed: bool,
    pub steps: u32,
    pub cost_usd_micros: i64,
}

/// The spawner: creates child streams in the parent's log root.
pub struct Spawner {
    log_root: PathBuf,
    kernel_config: PathBuf,
    max_steps: u32,
    feedback: bool,
}

impl Spawner {
    pub fn new(log_root: &Path, kernel_config: &Path, feedback: bool, max_steps: u32) -> Self {
        Self {
            log_root: log_root.to_path_buf(),
            kernel_config: kernel_config.to_path_buf(),
            max_steps,
            feedback,
        }
    }

    /// Spawn a child: append a Spawn event naming the child stream_id to the
    /// parent stream, create the child stream in the same log root.
    /// Returns the child handle plus delegation overhead in milliseconds.
    pub fn spawn(
        &self,
        _parent_log: &Path,
        _parent_stream: uuid::Uuid,
        mission: &str,
    ) -> (Child, f64) {
        let _ = (self, mission);
        unimplemented!("gate 5 red")
    }

    /// Drive a child to completion on this thread (parallel = N threads).
    pub fn run_to_completion(&self, _child: &Child) -> ChildReport {
        unimplemented!("gate 5 red")
    }
}
