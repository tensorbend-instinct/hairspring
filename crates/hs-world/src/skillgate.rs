//! Held-out and control evaluation runner for the skill install gate.
//!
//! A `Trial` is a verifier command. Exit status 0 means the verifier passed.
//! Each trial is run twice: once WITH the candidate skill (the skill body is
//! written to a file whose path is in `HS_SKILL_PATH`) and once as a BASELINE
//! (no `HS_SKILL_PATH`, `HS_SKILL_BASELINE=1`). The four outcomes become the
//! `SkillGateEvidence` that `install_skill_gated` judges. The runner executes
//! the verifier itself, so the evidence is not caller-supplied.
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Trial {
    /// Shell command; run with `sh -c` in `cwd`.
    pub command: String,
    pub cwd: PathBuf,
    pub timeout: Duration,
}

impl Trial {
    pub fn new(command: impl Into<String>, cwd: impl Into<PathBuf>) -> Self {
        Trial { command: command.into(), cwd: cwd.into(), timeout: Duration::from_secs(120) }
    }

    /// Run the verifier. A timeout, spawn failure or non-zero exit is a fail.
    pub fn run(&self, skill_path: Option<&Path>) -> bool {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(&self.command).current_dir(&self.cwd).stdout(Stdio::null()).stderr(Stdio::null());
        match skill_path {
            Some(p) => {
                cmd.env("HS_SKILL_PATH", p).env_remove("HS_SKILL_BASELINE");
            }
            None => {
                cmd.env_remove("HS_SKILL_PATH").env("HS_SKILL_BASELINE", "1");
            }
        }
        let Ok(mut child) = cmd.spawn() else { return false };
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(st)) => return st.success(),
                Ok(None) if start.elapsed() < self.timeout => std::thread::sleep(Duration::from_millis(10)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
            }
        }
    }
}
