//! Filesystem-native outer-loop harness optimization, following Lee et al.
//! Meta-Harness. The filesystem is the proposer interface: each iteration
//! preserves candidate source, raw trial traces, scores, and reflections.
//! Evaluation is injected and external to candidate code.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct SearchConfig {
    pub iterations: u32,
    pub trials_per_task: u32,
    pub search_tasks: Vec<String>,
    pub baseline_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CandidateProposal {
    pub name: String,
    pub parent: String,
    pub hypothesis: String,
    pub reflection: String,
    pub files: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct TrialOutcome {
    pub task: String,
    pub trial: u32,
    pub passed: bool,
    pub score: f64,
    pub trace: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrontierCandidate {
    pub name: String,
    pub iteration: u32,
    pub mean_score: f64,
    pub parent: String,
}

#[derive(Clone, Debug)]
pub struct SearchResult {
    pub frontier: FrontierCandidate,
    pub root: PathBuf,
}

#[derive(Debug)]
pub enum MetaHarnessError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Invalid(String),
}
impl From<std::io::Error> for MetaHarnessError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<serde_json::Error> for MetaHarnessError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}
impl std::fmt::Display for MetaHarnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "filesystem: {e}"),
            Self::Json(e) => write!(f, "json: {e}"),
            Self::Invalid(e) => write!(f, "invalid meta-harness state: {e}"),
        }
    }
}
impl std::error::Error for MetaHarnessError {}

pub struct MetaHarness {
    root: PathBuf,
    config: SearchConfig,
}

impl MetaHarness {
    #[must_use]
    pub fn new(root: &Path, config: SearchConfig) -> Self {
        Self {
            root: root.to_path_buf(),
            config,
        }
    }

    pub fn run<P, E>(
        &self,
        mut proposer: P,
        mut evaluator: E,
    ) -> Result<SearchResult, MetaHarnessError>
    where
        P: FnMut(u32, &Path) -> CandidateProposal,
        E: FnMut(&CandidateProposal, &str, u32) -> TrialOutcome,
    {
        if self.config.search_tasks.is_empty() || self.config.trials_per_task == 0 {
            return Err(MetaHarnessError::Invalid(
                "search tasks and trials must be non-empty".into(),
            ));
        }
        fs::create_dir_all(self.root.join("iterations"))?;
        self.write_manifest()?;
        let start = self.next_iteration()?;
        let mut frontier = self.read_frontier()?.unwrap_or(FrontierCandidate {
            name: self.config.baseline_name.clone(),
            iteration: 0,
            mean_score: 0.0,
            parent: String::new(),
        });
        for offset in 0..self.config.iterations {
            let iteration = start + offset;
            let proposal = proposer(iteration, &self.root);
            validate_proposal(&proposal)?;
            let cand_root = self.root.join(format!(
                "iterations/{iteration:04}/candidates/{}",
                proposal.name
            ));
            let source_root = cand_root.join("source");
            fs::create_dir_all(&source_root)?;
            for (relative, content) in &proposal.files {
                let path = safe_source_path(&source_root, relative)?;
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(path, content)?;
            }
            fs::write(
                cand_root.join("proposal.json"),
                serde_json::to_vec_pretty(&proposal)?,
            )?;
            fs::write(cand_root.join("reflection.md"), &proposal.reflection)?;

            let mut sum = 0.0;
            let mut count = 0_u64;
            let mut passed = 0_u64;
            for task in &self.config.search_tasks {
                for trial in 1..=self.config.trials_per_task {
                    let raw = evaluator(&proposal, task, trial);
                    let valid = raw.task == *task
                        && raw.trial == trial
                        && raw.score.is_finite()
                        && (0.0..=1.0).contains(&raw.score)
                        && !raw.trace.trim().is_empty();
                    let score = if valid { raw.score } else { 0.0 };
                    let pass = valid && raw.passed && score > 0.0;
                    sum += score;
                    count += 1;
                    if pass {
                        passed += 1;
                    }
                    let trial_root = cand_root.join(format!("trials/{task}/{trial:04}"));
                    fs::create_dir_all(&trial_root)?;
                    fs::write(trial_root.join("trace.log"), &raw.trace)?;
                    fs::write(
                        trial_root.join("result.json"),
                        serde_json::to_vec_pretty(&serde_json::json!({
                            "task": task, "trial": trial, "passed": pass, "score": score,
                            "valid_evidence": valid, "error": raw.error,
                        }))?,
                    )?;
                }
            }
            let mean = if count == 0 { 0.0 } else { sum / count as f64 };
            fs::write(
                cand_root.join("score.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "mean_score": mean, "passes": passed, "trials": count,
                }))?,
            )?;
            let candidate = FrontierCandidate {
                name: proposal.name.clone(),
                iteration,
                mean_score: mean,
                parent: proposal.parent.clone(),
            };
            if better(&candidate, &frontier) {
                frontier = candidate.clone();
            }
            self.append_summary(&candidate, &proposal, &frontier)?;
            fs::write(
                self.root.join("frontier.json"),
                serde_json::to_vec_pretty(&frontier)?,
            )?;
        }
        Ok(SearchResult {
            frontier,
            root: self.root.clone(),
        })
    }

    fn write_manifest(&self) -> Result<(), MetaHarnessError> {
        let p = self.root.join("search_config.json");
        if !p.exists() {
            fs::write(
                p,
                serde_json::to_vec_pretty(&serde_json::json!({
                    "baseline": self.config.baseline_name,
                    "trials_per_task": self.config.trials_per_task,
                    "search_tasks": self.config.search_tasks,
                }))?,
            )?;
        }
        Ok(())
    }

    fn next_iteration(&self) -> Result<u32, MetaHarnessError> {
        let mut max = 0;
        for entry in fs::read_dir(self.root.join("iterations"))? {
            let name = entry?.file_name();
            if let Some(n) = name.to_str().and_then(|s| s.parse::<u32>().ok()) {
                max = max.max(n);
            }
        }
        Ok(max + 1)
    }

    fn read_frontier(&self) -> Result<Option<FrontierCandidate>, MetaHarnessError> {
        let p = self.root.join("frontier.json");
        if !p.exists() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_slice(&fs::read(p)?)?))
    }

    fn append_summary(
        &self,
        candidate: &FrontierCandidate,
        proposal: &CandidateProposal,
        frontier: &FrontierCandidate,
    ) -> Result<(), MetaHarnessError> {
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join("evolution_summary.jsonl"))?;
        writeln!(
            f,
            "{}",
            serde_json::to_string(&serde_json::json!({
                "iteration": candidate.iteration, "candidate": candidate.name,
                "parent": candidate.parent, "hypothesis": proposal.hypothesis,
                "mean_score": candidate.mean_score, "frontier": frontier.name,
            }))?
        )?;
        Ok(())
    }
}

fn better(candidate: &FrontierCandidate, frontier: &FrontierCandidate) -> bool {
    candidate.mean_score > frontier.mean_score
}

fn validate_proposal(p: &CandidateProposal) -> Result<(), MetaHarnessError> {
    if p.name.trim().is_empty() || p.files.is_empty() {
        return Err(MetaHarnessError::Invalid(
            "candidate needs a name and source files".into(),
        ));
    }
    if p.name.contains('/') || p.name.contains("..") {
        return Err(MetaHarnessError::Invalid(
            "candidate name escapes its iteration".into(),
        ));
    }
    Ok(())
}

fn safe_source_path(root: &Path, relative: &str) -> Result<PathBuf, MetaHarnessError> {
    let rel = Path::new(relative);
    if rel.is_absolute()
        || rel
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(MetaHarnessError::Invalid(format!(
            "source path escapes candidate: {relative}"
        )));
    }
    Ok(root.join(rel))
}
