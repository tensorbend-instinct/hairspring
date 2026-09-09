//! HAIRSPRING gate 8, benchmark half: SWE-bench Verified runner plumbing.
//!
//! - `load_jsonl` parses the SWE-bench Verified dataset shape (one JSON
//!   object per line: `instance_id`, repo, `base_commit`, `problem_statement`,
//!   patch, `FAIL_TO_PASS`, `PASS_TO_PASS`).
//! - `BenchRunner::run_fixture` executes the offline plumbing proof: fixture
//!   "repos" are directories with a code file and a check script; patch
//!   sources are scripted (gold / wrong / budget-burn) so the proof needs
//!   NO paid model calls. Real missions route through hs-loop with the
//!   per-mission budget enforced by `InnerLoop::set_budget_micros` (proven in
//!   hs-loop/tests/budget.rs); this runner enforces the same cap at the
//!   benchmark level and scores kills as failures.
//! - `BenchReport::to_swebench_json` emits the SWE-bench report shape
//!   (resolved / unresolved / `no_apply` lists) plus our `budget_killed` list.

use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug)]
pub enum BenchError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Missing(String),
}
impl From<std::io::Error> for BenchError {
    fn from(e: std::io::Error) -> Self {
        BenchError::Io(e)
    }
}
impl From<serde_json::Error> for BenchError {
    fn from(e: serde_json::Error) -> Self {
        BenchError::Json(e)
    }
}

/// One SWE-bench Verified dataset row.
#[derive(Clone, Debug, Deserialize)]
pub struct BenchInstance {
    pub instance_id: String,
    pub repo: String,
    pub base_commit: String,
    pub problem_statement: String,
    pub patch: String,
    #[serde(rename = "FAIL_TO_PASS")]
    pub fail_to_pass: Vec<String>,
    #[serde(rename = "PASS_TO_PASS", default)]
    pub pass_to_pass: Vec<String>,
}

pub fn load_jsonl(path: &Path) -> Result<Vec<BenchInstance>, BenchError> {
    let text = std::fs::read_to_string(path)?;
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).map_err(BenchError::Json))
        .collect()
}

/// Which mission arm: System = harness on (feedback + self-mod); Baseline =
/// same loop with feedback and self-mod disabled (realbench ON/OFF shape).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Arm {
    Baseline,
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Resolved,
    Unresolved,
    /// Killed at the per-mission USD cap; scores as failure.
    BudgetKilled,
    /// Never launched: the run-level cap was spent first. Reported, not dropped.
    NotRun,
}

/// Scripted patch source for the offline plumbing proof.
#[derive(Clone, Copy, Debug)]
pub enum PatchSource {
    /// Apply the instance's own gold patch: must resolve.
    Gold,
    /// Apply a deliberately wrong patch: must NOT resolve (eval's teeth).
    Wrong,
    /// Burn provider-reported cost until the cap fires: must be killed.
    BudgetBurn,
}

#[derive(Clone, Debug)]
pub struct InstanceResult {
    pub instance_id: String,
    pub arm: Arm,
    pub outcome: Outcome,
    pub cost_micros: u64,
}

/// Cost the scripted burn reports per model call (mirrors the hs-loop
/// benchmodel plugin's 900 micro-USD so both enforcers see the same shape).
const BURN_COST_PER_CALL_MICROS: u64 = 900;

pub struct BenchRunner {
    root: PathBuf,
    budget_cap_micros: u64,
}

impl BenchRunner {
    #[must_use]
    pub fn new(root: &Path, budget_cap_micros: u64) -> Self {
        BenchRunner {
            root: root.to_path_buf(),
            budget_cap_micros,
        }
    }

    /// Run one fixture instance offline. The fixture "repo" is a directory
    /// holding code.txt + check.sh; resolution = check.sh exits 0 after the
    /// patch is applied.
    pub fn run_fixture(
        &self,
        inst: &BenchInstance,
        src: PatchSource,
        arm: Arm,
    ) -> Result<InstanceResult, BenchError> {
        let dir = self.root.join(format!(
            "{}-{}",
            inst.instance_id.replace('/', "_"),
            arm_name(arm)
        ));
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("code.txt"), "broken\n")?;

        // The fixture's FAIL_TO_PASS check: code.txt must contain the word
        // named in the problem statement.
        let expected = inst
            .problem_statement
            .rsplit("word ")
            .next()
            .and_then(|s| s.split_whitespace().next())
            .ok_or_else(|| BenchError::Missing("expected word in problem statement".into()))?
            .to_string();
        std::fs::write(
            dir.join("check.sh"),
            format!("#!/bin/sh\ngrep -q '^{expected}$' code.txt\n"),
        )?;

        let mut cost_micros = 0u64;
        match src {
            PatchSource::Gold => {
                // apply the gold patch: take the added line
                let added = inst
                    .patch
                    .lines()
                    .find(|l| l.starts_with('+') && !l.starts_with("+++"))
                    .ok_or_else(|| BenchError::Missing("added line in gold patch".into()))?;
                std::fs::write(dir.join("code.txt"), format!("{}\n", &added[1..]))?;
            }
            PatchSource::Wrong => {
                std::fs::write(dir.join("code.txt"), "wrong\n")?;
            }
            PatchSource::BudgetBurn => {
                // scripted burn: each "model call" reports cost; the runner's
                // enforcer kills the mission the moment the cap is exceeded
                loop {
                    cost_micros += BURN_COST_PER_CALL_MICROS;
                    if cost_micros > self.budget_cap_micros {
                        return Ok(InstanceResult {
                            instance_id: inst.instance_id.clone(),
                            arm,
                            outcome: Outcome::BudgetKilled,
                            cost_micros,
                        });
                    }
                    if cost_micros > 1_000_000_000 {
                        return Err(BenchError::Missing(
                            "budget enforcer never fired: cap not enforced".into(),
                        ));
                    }
                }
            }
        }

        let status = Command::new("sh")
            .arg("check.sh")
            .current_dir(&dir)
            .status()?;
        Ok(InstanceResult {
            instance_id: inst.instance_id.clone(),
            arm,
            outcome: if status.success() {
                Outcome::Resolved
            } else {
                Outcome::Unresolved
            },
            cost_micros,
        })
    }
}

fn arm_name(arm: Arm) -> &'static str {
    match arm {
        Arm::Baseline => "baseline",
        Arm::System => "system",
    }
}

/// Benchmark report in the SWE-bench shape.
pub struct BenchReport {
    results: Vec<InstanceResult>,
}
impl BenchReport {
    #[must_use]
    pub fn new(results: Vec<InstanceResult>) -> Self {
        BenchReport { results }
    }
    #[must_use]
    pub fn not_run_count(&self) -> usize {
        self.results
            .iter()
            .filter(|r| r.outcome == Outcome::NotRun)
            .count()
    }
    #[must_use]
    pub fn total_cost_micros(&self) -> u64 {
        self.results.iter().map(|r| r.cost_micros).sum()
    }
    #[must_use]
    pub fn resolved_count(&self, arm: Arm) -> usize {
        self.results
            .iter()
            .filter(|r| r.arm == arm && r.outcome == Outcome::Resolved)
            .count()
    }
    /// SWE-bench report: resolved / unresolved / `no_apply` id lists, plus our
    /// `budget_killed` list (kills are failures, reported separately so the
    /// record shows WHY they failed).
    #[must_use]
    pub fn to_swebench_json(&self, model: &str) -> serde_json::Value {
        let ids = |arm: Arm, o: Outcome| {
            self.results
                .iter()
                .filter(|r| r.arm == arm && r.outcome == o)
                .map(|r| r.instance_id.clone())
                .collect::<Vec<_>>()
        };
        serde_json::json!({
            "model": model,
            "resolved": ids(Arm::System, Outcome::Resolved),
            "unresolved": ids(Arm::System, Outcome::Unresolved),
            "no_apply": [],
            "budget_killed": ids(Arm::System, Outcome::BudgetKilled),
            "not_run": ids(Arm::System, Outcome::NotRun),
            "baseline": {
                "resolved": ids(Arm::Baseline, Outcome::Resolved),
                "unresolved": ids(Arm::Baseline, Outcome::Unresolved),
                "budget_killed": ids(Arm::Baseline, Outcome::BudgetKilled),
            },
        })
    }
}

// ------------------------------------------------------- mission adapter ---

/// Prepare a mission workspace: clone the instance's repo at its base
/// commit into `cache_dir`/<`instance_id`> and drop the problem statement in as
/// `problem_statement.md`. Local paths and file:// URLs both work (the
/// offline proof uses local fixture repos; the real run uses cached GitHub
/// mirrors).
pub fn prep_workspace(inst: &BenchInstance, cache_dir: &Path) -> Result<PathBuf, BenchError> {
    std::fs::create_dir_all(cache_dir)?;
    let ws = cache_dir.join(inst.instance_id.replace(['/', '\\'], "_"));
    if ws.exists() {
        std::fs::remove_dir_all(&ws)?;
    }
    let st = Command::new("git")
        .args(["clone", "-q", &inst.repo])
        .arg(&ws)
        .status()?;
    if !st.success() {
        return Err(BenchError::Missing(format!(
            "git clone {} failed",
            inst.repo
        )));
    }
    let st = Command::new("git")
        .args(["checkout", "-q", &inst.base_commit])
        .current_dir(&ws)
        .status()?;
    if !st.success() {
        return Err(BenchError::Missing(format!(
            "git checkout {} failed",
            inst.base_commit
        )));
    }
    std::fs::write(ws.join("problem_statement.md"), &inst.problem_statement)?;
    Ok(ws)
}

/// Patch-application taxonomy: a patch that git cannot apply is `NoApply` -
/// reported separately from test failures, exactly like SWE-bench.
#[derive(Debug)]
pub enum ApplyResult {
    Applied,
    NoApply(String),
}

pub fn apply_model_patch(workspace: &Path, patch: &str) -> Result<ApplyResult, BenchError> {
    if patch.trim().is_empty() {
        return Ok(ApplyResult::NoApply("empty patch".into()));
    }
    // The temp patch file is harness machinery: it must live OUTSIDE the
    // agent-visible workspace (a crash mid-cleanup once stranded it in-tree,
    // where agents saw it and the candidate diff grew a bogus deletion hunk).
    let patch_path = std::env::temp_dir().join(format!(
        ".hs-eval-{}-{}.patch",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    std::fs::write(&patch_path, patch)?;
    let out = Command::new("git")
        .args(["apply", "--whitespace=nowarn"])
        .arg(&patch_path)
        .current_dir(workspace)
        .output();
    let _ = std::fs::remove_file(&patch_path);
    let out = out?;
    if out.status.success() {
        Ok(ApplyResult::Applied)
    } else {
        Ok(ApplyResult::NoApply(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

/// Per-test results for `FAIL_TO_PASS` + `PASS_TO_PASS`.
#[derive(Debug)]
pub struct TestOutcome {
    pub results: Vec<(String, bool)>,
}
impl TestOutcome {
    #[must_use]
    pub fn all_passing(&self) -> bool {
        !self.results.is_empty() && self.results.iter().all(|(_, ok)| *ok)
    }
}

/// Run each test script with the workspace as cwd; exit 0 = pass.
pub fn run_tests(
    workspace: &Path,
    fail_to_pass: &[String],
    pass_to_pass: &[String],
) -> Result<TestOutcome, BenchError> {
    let mut results = vec![];
    for t in fail_to_pass.iter().chain(pass_to_pass.iter()) {
        let st = Command::new("sh").arg(t).current_dir(workspace).status()?;
        results.push((t.clone(), st.success()));
    }
    Ok(TestOutcome { results })
}

// ------------------------------------------------------ patch extraction ---

/// Extract a unified diff from a model completion. Models wrap diffs in
/// prose and markdown fences; we accept a `diff` fence, any fence whose
/// body starts with a diff header, or a bare diff in the text. Prose-only
/// completions yield None (the runner treats that as `NoApply` - we never
/// invent a patch).
/// Terminate an extracted diff WITHOUT trimming trailing whitespace lines:
/// a hunk's final context line can be a single space (a blank source line)
/// and trimming it corrupts the hunk ("error: corrupt patch at line N",
/// octodns-1298 2026-09-07). Strip only leading whitespace and the trailing
/// newline run, then re-terminate exactly once.
fn terminate_diff(body: &str) -> String {
    format!("{}\n", body.trim_start().trim_end_matches('\n'))
}

#[must_use]
pub fn extract_patch(completion: &str) -> Option<String> {
    // 1. fenced blocks, preferring `diff`
    let mut fences: Vec<&str> = vec![];
    let mut rest = completion;
    while let Some(start) = rest.find("```") {
        let after_tick = &rest[start + 3..];
        let body_and_on = match after_tick.find('\n') {
            Some(nl) => &after_tick[nl + 1..],
            None => break,
        };
        let Some(end) = body_and_on.find("```") else {
            break;
        };
        let lang = after_tick[..after_tick.find('\n').unwrap()].trim();
        let body = &body_and_on[..end];
        if lang == "diff" {
            return Some(terminate_diff(body));
        }
        fences.push(body);
        rest = &body_and_on[end + 3..];
    }
    for body in fences {
        if body.trim_start().starts_with("--- a/") || body.trim_start().starts_with("diff --git") {
            return Some(terminate_diff(body));
        }
    }
    // 2. bare diff anywhere in the text: from the first "--- a/" line to the end
    if let Some(pos) = completion.find("\n--- a/") {
        return Some(terminate_diff(&completion[pos + 1..]));
    }
    if completion.trim_start().starts_with("--- a/") {
        return Some(terminate_diff(completion));
    }
    None
}

// --------------------------------------------------------- orchestration ---

/// A mission executor: the real one drives hs-loop missions against paid
/// models (parked until budget approval); tests use scripted executors.
pub trait MissionExec {
    fn run_mission(
        &self,
        inst: &BenchInstance,
        arm: Arm,
        mission_cap_micros: u64,
    ) -> Result<InstanceResult, BenchError>;
}

/// Run a set of instances on one arm under TWO budgets: the per-mission cap
/// (enforced by the executor / hs-loop) and the run-level cap, which stops
/// launching new missions once spent. Remaining instances are marked `NotRun`
/// in the report - visible, never silently dropped.
pub fn run_set<E: MissionExec>(
    exec: &E,
    instances: &[BenchInstance],
    arm: Arm,
    run_cap_micros: u64,
    mission_cap_micros: u64,
) -> BenchReport {
    let mut results = vec![];
    let mut spent = 0u64;
    for inst in instances {
        if spent >= run_cap_micros {
            results.push(InstanceResult {
                instance_id: inst.instance_id.clone(),
                arm,
                outcome: Outcome::NotRun,
                cost_micros: 0,
            });
            continue;
        }
        match exec.run_mission(inst, arm, mission_cap_micros) {
            Ok(r) => {
                spent += r.cost_micros;
                results.push(r);
            }
            Err(_) => results.push(InstanceResult {
                instance_id: inst.instance_id.clone(),
                arm,
                outcome: Outcome::Unresolved,
                cost_micros: 0,
            }),
        }
    }
    BenchReport::new(results)
}

// ------------------------------------------------------- mission prompt ---

/// Build the mission prompt for one instance: problem statement, workspace
/// layout (tracked files, two levels), and the response contract - exactly
/// one fenced `diff` block that `git apply` accepts. The contract is what
/// `extract_patch` parses back, proven round-trip in the tests.
#[must_use]
pub fn mission_prompt(inst: &BenchInstance, workspace: &Path) -> String {
    let mut files: Vec<String> = vec![];
    let mut stack = vec![workspace.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                let rel = p
                    .strip_prefix(workspace)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .to_string();
                if rel.starts_with(".git") || rel == "problem_statement.md" {
                    continue;
                }
                if p.is_dir() {
                    stack.push(p);
                } else {
                    files.push(rel);
                }
            }
        }
    }
    files.sort();
    format!(
        "MISSION: {id}\n\n\
         You are modifying a checked-out repository. Fix the issue below with \
         a minimal change.\n\n\
         PROBLEM STATEMENT:\n{statement}\n\n\
         WORKSPACE FILES:\n{layout}\n\n\
         RESPONSE CONTRACT: reply with exactly one ```diff fenced block \
         containing a unified diff (paths a/... b/...) that `git apply` \
         accepts. No other fences. No commits. No new dependencies.\n",
        id = inst.instance_id,
        statement = inst.problem_statement.trim(),
        layout = files.join("\n")
    )
}
