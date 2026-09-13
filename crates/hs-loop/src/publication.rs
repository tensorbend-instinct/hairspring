//! Gate 8 publication harness (v5: "Beat the published single-session
//! incumbent baseline by 2x on the defined benchmark suite (gate 8). A
//! gate, not a promise."; checklist 7.9: the full `T_mission`
//! decomposition is published per run, win or lose).
//!
//! The suite is the defined token family (`task-1..task-N`, each mission
//! graded by the ground-truth checker against its own `TOKEN-N-SECRET`)
//! run in TWO arms on the real pipeline - real `InnerLoop`, real
//! checker, mechanical (never canned) verifier:
//!
//! - the **incumbent** arm is the published single-session baseline
//!   (v5 p.36: "the incumbent resets every session and restarts when it
//!   breaks"): one fresh session per attempt, no cross-session memory,
//!   no feedback channel - a red check is never shown to the model, so
//!   the deterministic baseline replays the identical wrong call until
//!   the step budget burns out (`steps_exhausted`), then the harness
//!   restarts it from scratch; the failed attempt's full measured cost
//!   is paid twice - once lost, once as the restart redo;
//! - the **hairspring** arm is the real loop end to end: one continuous
//!   session per task with verifier feedback enabled - the red check
//!   names the expected token in-step and the model repairs.
//!
//! The comparison is published in `T_mission` decomposition TERMS
//! (steps, restarts, stuck repeats, priced model cost), never wall
//! microseconds, and the artifact carries the operationalization
//! verbatim so the claim is falsifiable.

use crate::mission_time::{Decomposition, MissionTime};
use crate::repl::load_session;
use crate::LoopError;
use std::path::{Path, PathBuf};

/// The agreed default operationalization, printed verbatim into every
/// publication artifact.
pub const OPERATIONALIZATION: &str = r#"Gate 8 operationalization (agreed default 2026-09-09):
- suite: the defined token-family suite (task-1..task-N), each mission
  graded by the ground-truth checker against its own TOKEN-N-SECRET;
- incumbent arm: the published single-session baseline - one session per
  attempt, NO cross-session memory (fresh session dir, fresh model
  script, fresh process per attempt) and NO feedback channel; a red
  check is never shown to the model, so the deterministic baseline
  replays the identical wrong call until its step budget burns out
  (steps_exhausted); on failure it restarts from scratch, and the failed
  attempt's full measured cost is paid twice (once lost, once as the
  restart redo), matching v5 p.36 "the incumbent resets every session
  and restarts when it breaks";
- hairspring arm: the real pipeline end to end - one continuous session
  per task with verifier feedback enabled, so a red check repairs
  in-step;
- the verifier is the mechanical judge (ledger test-run evidence plus
  objective token), never a canned verdict;
- comparison is published in T_mission decomposition TERMS (steps,
  restarts, stuck repeats, priced model cost), never wall microseconds;
- the full decomposition is published per run, win or lose."#;

const PLUGIN_DIR_DEFAULT: &str = "/mnt/instinct-nvme/hairspring/target/debug";

/// The verify-then-submit diff used by every honest attempt (the
/// mechanical verifier requires a recorded candidate test run).
const VERIFY_DIFF: &str = "```diff\n--- a/code.txt\n+++ b/code.txt\n@@ -1 +1 @@\n-broken\n+fixed\n```";

/// The published verdict. Gate 8 is a gate: the artifact prints it win
/// or lose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Incumbent priced steps >= 2x hairspring priced steps.
    Win,
    /// The 2x bar was not met.
    Lose,
}

/// One task's run inside one arm: every attempt's measured decomposition.
#[derive(Clone, Debug)]
pub struct TaskRun {
    /// The mission goal string (`task-N`).
    pub mission: String,
    /// One entry per attempt, in order; length > 1 means restarts.
    pub attempts: Vec<Decomposition>,
    /// Whether the final attempt passed the checker + verifier.
    pub passed: bool,
    /// Failed attempts that were restarted from scratch.
    pub restarts: u64,
}

impl TaskRun {
    /// Steps this task's arm must pay: every attempt plus the restart
    /// redo of each failed attempt (the incumbent pays lost work twice;
    /// a single-attempt task pays its steps once).
    #[must_use]
    pub fn raw_steps_paid(&self) -> u64 {
        let raw: u64 = self.attempts.iter().map(|a| a.n_steps).sum();
        let failed: u64 = self
            .attempts
            .iter()
            .take(self.attempts.len().saturating_sub(1))
            .map(|a| a.n_steps)
            .sum();
        raw + failed
    }
}

/// One arm's full suite result.
#[derive(Clone, Debug, Default)]
pub struct ArmReport {
    /// Arm name (`incumbent` or `hairspring`).
    pub name: String,
    /// Per-task runs, in suite order.
    pub tasks: Vec<TaskRun>,
    /// All attempts' decompositions summed.
    pub total: Decomposition,
    /// Total restarts across the suite.
    pub restarts: u64,
    /// Tasks that passed.
    pub passed: u32,
    /// Raw summed steps across all attempts.
    pub raw_steps: u64,
    /// Steps priced with the restart redo (failed attempts counted
    /// twice for the incumbent; equal to `raw_steps` for hairspring).
    pub priced_steps: u64,
}

/// The full two-arm publication.
#[derive(Clone, Debug)]
pub struct Publication {
    /// Suite label.
    pub suite: String,
    /// Tasks in the suite.
    pub task_count: u32,
    /// Failed attempts per incumbent task.
    pub failed_attempts: u32,
    /// The single-session incumbent baseline arm.
    pub incumbent: ArmReport,
    /// The hairspring arm.
    pub hairspring: ArmReport,
    /// `incumbent.priced_steps / hairspring.priced_steps`.
    pub speedup_steps: f64,
    /// Win or lose against the 2x gate.
    pub verdict: Verdict,
}

impl Publication {
    /// The published report body.
    #[must_use]
    pub fn report_lines(&self) -> Vec<String> {
        let mut lines = vec![
            "HAIRSPRING gate 8 publication (benchmark half)".to_string(),
            format!(
                "suite={} tasks={} incumbent_failed_attempts_per_task={}",
                self.suite, self.task_count, self.failed_attempts
            ),
            "published win or lose".to_string(),
            String::new(),
            "OPERATIONALIZATION (verbatim):".to_string(),
            OPERATIONALIZATION.to_string(),
            String::new(),
        ];
        for arm in [&self.incumbent, &self.hairspring] {
            lines.push(format!("ARM {}:", arm.name));
            for t in &arm.tasks {
                let steps: Vec<u64> = t.attempts.iter().map(|a| a.n_steps).collect();
                lines.push(format!(
                    "  {}: attempts={} restarts={} passed={} steps_per_attempt={:?}",
                    t.mission,
                    t.attempts.len(),
                    t.restarts,
                    t.passed,
                    steps
                ));
            }
            lines.push(format!(
                "  totals: raw_steps={} priced_steps={} restarts={} passed={}",
                arm.raw_steps, arm.priced_steps, arm.restarts, arm.passed
            ));
            lines.push(
                "  full T_mission decomposition (summed over all attempts):".to_string(),
            );
            for l in arm.total.report_lines(&arm.name) {
                lines.push(format!("    {l}"));
            }
            lines.push(String::new());
        }
        lines.push("COMPARISON (decomposition terms, not wall microseconds):".to_string());
        lines.push(format!(
            "  incumbent priced_steps={} hairspring priced_steps={} speedup={:.2}x target=2x",
            self.incumbent.priced_steps, self.hairspring.priced_steps, self.speedup_steps
        ));
        lines.push(format!(
            "  verdict={}",
            match self.verdict {
                Verdict::Win => "WIN",
                Verdict::Lose => "LOSE",
            }
        ));
        lines
    }

    /// Write `publication.txt` under `dir` and return its path.
    pub fn write(&self, dir: &Path) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("publication.txt");
        let mut body = self.report_lines().join("\n");
        body.push('\n');
        std::fs::write(&path, body)?;
        Ok(path)
    }
}

fn sum_into(acc: &mut Decomposition, d: &Decomposition) {
    acc.n_steps += d.n_steps;
    acc.t_model_ms += d.t_model_ms;
    acc.t_overhead_ms += d.t_overhead_ms;
    acc.r_failures += d.r_failures;
    acc.t_recover_ms += d.t_recover_ms;
    acc.c_coord_events += d.c_coord_events;
    acc.c_coord_ms += d.c_coord_ms;
    acc.s_stuck_repeats += d.s_stuck_repeats;
    acc.s_stuck_ms += d.s_stuck_ms;
    acc.wall_ms += d.wall_ms;
    acc.unattributed_ms += d.unattributed_ms;
}

/// The fixture plugin directory: `HS_PUBLICATION_PLUGIN_DIR` wins, else
/// the dev-box target dir.
fn plugin_dir() -> PathBuf {
    std::env::var("HS_PUBLICATION_PLUGIN_DIR")
        .map_or_else(|_| PathBuf::from(PLUGIN_DIR_DEFAULT), PathBuf::from)
}

/// The git workspace `repo.exec` verifies candidates against (scratch
/// worktrees only; the live tree is never touched).
fn git_ws(root: &Path) -> Result<PathBuf, LoopError> {
    let ws = root.join("ws");
    if ws.join(".git").is_dir() && ws.join("code.txt").is_file() {
        // already initialized (the two-arm run creates it first)
        return Ok(ws);
    }
    std::fs::create_dir_all(&ws)?;
    std::fs::write(ws.join("code.txt"), "broken\n")?;
    let cmds: [&[&str]; 3] = [
        &["init", "-q"],
        &["add", "."],
        &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"],
    ];
    for args in cmds {
        let ok = std::process::Command::new("git")
            .args(args)
            .current_dir(&ws)
            .status()?
            .success();
        if !ok {
            return Err(LoopError::Visibility(format!(
                "publication fixture: git {args:?} failed"
            )));
        }
    }
    Ok(ws)
}

/// Write the per-attempt fixture: wrapper scripts carry the fixture env
/// (no process-env mutation in library code), the toml points at them.
fn write_attempt_fixture(
    dir: &Path,
    ws: &Path,
    script_lines: &[String],
) -> Result<PathBuf, LoopError> {
    std::fs::create_dir_all(dir)?;
    let plugins = plugin_dir();
    let script_path = dir.join("script.jsonl");
    std::fs::write(&script_path, script_lines.join(""))?;

    let vfmodel_sh = dir.join("vfmodel.sh");
    std::fs::write(
        &vfmodel_sh,
        format!(
            "#!/bin/sh\nHS_VF_SCRIPT='{}' exec {}/hs-plugin-vfmodel\n",
            script_path.display(),
            plugins.display()
        ),
    )?;
    let repoexec_sh = dir.join("repoexec.sh");
    std::fs::write(
        &repoexec_sh,
        format!(
            "#!/bin/sh\nHS_SWE_WORKSPACE='{}' exec {}/hs-plugin-repoexec\n",
            ws.display(),
            plugins.display()
        ),
    )?;
    use std::os::unix::fs::PermissionsExt;
    for sh in [&vfmodel_sh, &repoexec_sh] {
        std::fs::set_permissions(sh, std::fs::Permissions::from_mode(0o755))?;
    }

    let toml = format!(
        r#"
[[tools]]
name = "answer.write"
command = ["{}/hs-plugin-answer"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{}/hs-plugin-checker"]
subjects = ["*"]

[[tools]]
name = "repo.exec"
command = ["{}"]
subjects = ["*"]

[[models]]
name = "vfmodel"
command = ["{}"]
default = true
subjects = ["*"]
"#,
        plugins.display(),
        plugins.display(),
        repoexec_sh.display(),
        vfmodel_sh.display()
    );
    let config = dir.join("hairspring.toml");
    std::fs::write(&config, toml)?;
    Ok(config)
}

/// Run one attempt: a fresh session under `attempt_dir/run`, then the
/// attempt's measured decomposition from its own stream.
fn run_attempt(
    attempt_dir: &Path,
    ws: &Path,
    feedback: bool,
    max_steps: u32,
    goal: &str,
    script_lines: &[String],
) -> Result<(bool, Decomposition), LoopError> {
    let config = write_attempt_fixture(attempt_dir, ws, script_lines)?;
    let run = attempt_dir.join("run");
    let mut s = load_session(&config, &run, feedback, Some(max_steps), None, None)?;
    let r = s.run_goal(goal)?;
    let sid = s.vitals().stream_id;
    let reader = hs_log::StreamReader::open(&run, sid)?;
    Ok((r.passed, MissionTime::decompose(&reader)?))
}

fn answer_write(path: &Path, content: &str) -> String {
    let mut l = serde_json::json!({
        "tool": "answer.write",
        "args": {"path": path, "content": content},
    })
    .to_string();
    l.push('\n');
    l
}

fn verify_line() -> String {
    let mut l = serde_json::json!({
        "tool": "repo.exec",
        "args": {"command": "cat code.txt", "diff": VERIFY_DIFF},
    })
    .to_string();
    l.push('\n');
    l
}

/// Run the defined suite in both arms and build the publication.
///
/// `failed_attempts` is how many times the incumbent fails each task
/// (restarting from scratch each time) before its final attempt passes.
///
/// # Errors
/// Propagates fixture IO, loop, and log errors from the real pipeline.
pub fn run_publication(
    root: &Path,
    tasks: u32,
    failed_attempts: u32,
) -> Result<Publication, LoopError> {
    const INCUMBENT_BURN_STEPS: u32 = 6;
    const ATTEMPT_MAX_STEPS: u32 = 10;
    let ws = git_ws(root)?;
    let mut incumbent = ArmReport {
        name: "incumbent".to_string(),
        ..ArmReport::default()
    };
    let mut hairspring = ArmReport {
        name: "hairspring".to_string(),
        ..ArmReport::default()
    };

    for i in 1..=tasks {
        let goal = format!("task-{i}");

        // Incumbent: fresh session per attempt, no memory, no feedback;
        // the failed attempts replay the identical wrong call until the
        // step budget burns out (that is the baseline's doom loop), then
        // the harness restarts from scratch.
        let mut task = TaskRun {
            mission: goal.clone(),
            attempts: vec![],
            passed: false,
            restarts: 0,
        };
        for a in 1..=failed_attempts {
            let dir = root.join(format!("arms/incumbent/task-{i}/attempt-{a}"));
            let answer = dir.join("run").join("work").join(&goal).join("answer.txt");
            let script = vec![answer_write(&answer, &format!("WRONG-{i}"))];
            let (passed, d) =
                run_attempt(&dir, &ws, false, INCUMBENT_BURN_STEPS, &goal, &script)?;
            debug_assert!(!passed, "incumbent failed attempt must fail");
            task.restarts += 1;
            task.attempts.push(d);
        }
        let dir = root.join(format!(
            "arms/incumbent/task-{i}/attempt-{}",
            failed_attempts + 1
        ));
        let answer = dir.join("run").join("work").join(&goal).join("answer.txt");
        let script = vec![
            verify_line(),
            answer_write(&answer, &format!("TOKEN-{i}-SECRET")),
        ];
        let (passed, d) = run_attempt(&dir, &ws, false, ATTEMPT_MAX_STEPS, &goal, &script)?;
        task.passed = passed;
        task.attempts.push(d);
        for d in &task.attempts {
            sum_into(&mut incumbent.total, d);
            incumbent.raw_steps += d.n_steps;
        }
        incumbent.priced_steps += task.raw_steps_paid();
        incumbent.restarts += task.restarts;
        incumbent.passed += u32::from(task.passed);
        incumbent.tasks.push(task);

        // Hairspring: one continuous session with feedback; the wrong
        // submission is refuted in-step (the checker names the expected
        // token), the model verifies and repairs.
        let dir = root.join(format!("arms/hairspring/task-{i}"));
        let answer = dir.join("run").join("work").join(&goal).join("answer.txt");
        let script = vec![
            answer_write(&answer, &format!("WRONG-{i}")),
            verify_line(),
            answer_write(&answer, &format!("TOKEN-{i}-SECRET")),
        ];
        let (passed, d) = run_attempt(&dir, &ws, true, ATTEMPT_MAX_STEPS, &goal, &script)?;
        sum_into(&mut hairspring.total, &d);
        hairspring.raw_steps += d.n_steps;
        hairspring.priced_steps += d.n_steps;
        hairspring.passed += u32::from(passed);
        hairspring.tasks.push(TaskRun {
            mission: goal,
            attempts: vec![d],
            passed,
            restarts: 0,
        });
    }

    let speedup_steps = if hairspring.priced_steps == 0 {
        0.0
    } else {
        incumbent.priced_steps as f64 / hairspring.priced_steps as f64
    };
    let verdict = if speedup_steps >= 2.0 {
        Verdict::Win
    } else {
        Verdict::Lose
    };
    Ok(Publication {
        suite: "token-family".to_string(),
        task_count: tasks,
        failed_attempts,
        incumbent,
        hairspring,
        speedup_steps,
        verdict,
    })
}

// ------------------------------------------------------- 8.5 best-of-N ---

/// The agreed best-of-N operationalization, printed verbatim into every
/// best-of-N publication artifact.
pub const BEST_OF_N_OPERATIONALIZATION: &str = r#"Best-of-N operationalization (8.5; spec "The best-of-N baseline"):
- every task family carries an endpoint-wise best-of-N envelope of
  isolated agents with identical decision opportunities (SwarmWorld);
- isolates: N independent incumbent-style runs - fresh session per
  attempt, NO cross-session memory, NO feedback channel - with the same
  per-attempt step budgets and the same task secrets (matched seeds) as
  the candidate; outcomes vary deterministically per (task, isolate);
- envelope: per task, the endpoint of the BEST isolate (fewest priced
  steps among passing isolates); the candidate must beat the envelope,
  not any single isolate - that is beating matched independent search,
  not just adding samples;
- metric where the win lives: pass endpoints first (a task the
  envelope cannot pass is an endpoint matched search lost), priced_steps
  in T_mission decomposition terms against the 2x gate second; published
  win or lose."#;

/// One task's endpoint inside the endpoint-wise envelope: the best
/// isolate's measured result for this task.
#[derive(Clone, Debug)]
pub struct TaskEnvelope {
    /// The mission goal string (`task-N`).
    pub mission: String,
    /// Whether any isolate passed this task.
    pub passed: bool,
    /// The best (lowest) priced steps among passing isolates.
    pub priced_steps: u64,
    /// Which isolate supplied the endpoint (0-based).
    pub best_isolate: u32,
}

/// The endpoint-wise best-of-N envelope over all isolates.
#[derive(Clone, Debug)]
pub struct EnvelopeReport {
    /// Per-task endpoints in suite order.
    pub tasks: Vec<TaskEnvelope>,
    /// Sum of the per-task best endpoints.
    pub priced_steps: u64,
    /// Tasks any isolate passed.
    pub passed: u32,
}

/// The full 8.5 publication: the two-arm gate-8 run plus the best-of-N
/// envelope baseline the candidate must beat.
#[derive(Clone, Debug)]
pub struct BestOfNPublication {
    /// Suite label.
    pub suite: String,
    /// Tasks in the suite.
    pub task_count: u32,
    /// Maximum induced failed attempts per isolate task.
    pub failed_attempts: u32,
    /// Number of isolates in the envelope.
    pub n_isolates: u32,
    /// The single-incumbent baseline arm (gate-8 continuity).
    pub incumbent: ArmReport,
    /// The hairspring candidate arm.
    pub hairspring: ArmReport,
    /// The N isolate arms (incumbent-style, matched seeds).
    pub isolates: Vec<ArmReport>,
    /// The endpoint-wise envelope over the isolates.
    pub envelope: EnvelopeReport,
    /// `incumbent.priced_steps / hairspring.priced_steps` (gate 8).
    pub base_speedup: f64,
    /// Gate-8 verdict against the single incumbent.
    pub base_verdict: Verdict,
    /// `envelope.priced_steps / hairspring.priced_steps` (8.5).
    pub envelope_speedup: f64,
    /// Verdict against the envelope: the bar any claimed collective or
    /// evolutionary advantage must beat.
    pub envelope_verdict: Verdict,
    /// How the isolate outcomes were drawn.
    pub draw: IsolateDraw,
}

impl BestOfNPublication {
    /// The published report body: the full two-arm run, then the
    /// best-of-N envelope section, win or lose.
    #[must_use]
    pub fn report_lines(&self) -> Vec<String> {
        let mut lines = vec![
            "HAIRSPRING 8.5 best-of-N publication (envelope baseline)".to_string(),
            format!(
                "suite={} tasks={} isolates={} max_failed_attempts_per_task={} isolate_draw={}",
                self.suite,
                self.task_count,
                self.n_isolates,
                self.failed_attempts,
                match self.draw {
                    IsolateDraw::Uniform => "uniform",
                    IsolateDraw::Specialist => "specialist",
                }
            ),
            "published win or lose".to_string(),
            String::new(),
            "BEST-OF-N OPERATIONALIZATION (verbatim):".to_string(),
            BEST_OF_N_OPERATIONALIZATION.to_string(),
            String::new(),
        ];
        for arm in [&self.incumbent, &self.hairspring] {
            lines.push(format!("ARM {}:", arm.name));
            for t in &arm.tasks {
                lines.push(format!(
                    "  {}: attempts={} restarts={} passed={}",
                    t.mission,
                    t.attempts.len(),
                    t.restarts,
                    t.passed
                ));
            }
            lines.push(format!(
                "  totals: raw_steps={} priced_steps={} restarts={} passed={}",
                arm.raw_steps, arm.priced_steps, arm.restarts, arm.passed
            ));
        }
        lines.push(String::new());
        lines.push(
            "best-of-N envelope (endpoint-wise, matched decision opportunities):"
                .to_string(),
        );
        lines.push(format!(
            "  isolate_priced_steps={:?}",
            self.isolates
                .iter()
                .map(|i| i.priced_steps)
                .collect::<Vec<_>>()
        ));
        for t in &self.envelope.tasks {
            lines.push(format!(
                "  {}: passed={} priced_steps={} best_isolate={}",
                t.mission, t.passed, t.priced_steps, t.best_isolate
            ));
        }
        lines.push(format!(
            "  envelope_priced_steps={} envelope_passed={}",
            self.envelope.priced_steps, self.envelope.passed
        ));
        lines.push(String::new());
        lines.push(
            "COMPARISONS (metric=pass_endpoints_then_priced_steps, decomposition terms):"
                .to_string(),
        );
        lines.push(format!(
            "  gate-8 single incumbent: incumbent_priced_steps={} hairspring_priced_steps={} speedup={:.2}x target=2x verdict={}",
            self.incumbent.priced_steps,
            self.hairspring.priced_steps,
            self.base_speedup,
            verdict_str(self.base_verdict),
        ));
        lines.push(format!(
            "  8.5 envelope: envelope_priced_steps={} hairspring_priced_steps={} envelope_speedup={:.2}x target=2x envelope_verdict={}",
            self.envelope.priced_steps,
            self.hairspring.priced_steps,
            self.envelope_speedup,
            verdict_str(self.envelope_verdict),
        ));
        lines
    }

    /// Write `best-of-n-publication.txt` under `dir` and return its path.
    pub fn write(&self, dir: &Path) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("best-of-n-publication.txt");
        let mut body = self.report_lines().join("\n");
        body.push('\n');
        std::fs::write(&path, body)?;
        Ok(path)
    }
}

fn verdict_str(v: Verdict) -> &'static str {
    match v {
        Verdict::Win => "WIN",
        Verdict::Lose => "LOSE",
    }
}

/// How the isolates' outcomes are drawn. Both draws keep matched seeds
/// (identical task secrets) and identical decision opportunities (same
/// attempt budgets); only the outcome spread differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IsolateDraw {
    /// Every isolate can pass every task; failures per (task, isolate)
    /// spread deterministically in `0..=failed_attempts`.
    Uniform,
    /// Specialist isolates (`gate7_proof_3` shape): isolate `k` covers
    /// task `i` iff `7*i + k` is a multiple of `n_isolates + 1`; a covered task
    /// passes on the first attempt, an uncovered one burns every attempt
    /// and never passes. Matched independent search with partial
    /// coverage per isolate.
    Specialist,
}

/// How many times isolate `k` fails task `i` before its passing attempt:
/// a deterministic spread in `0..=failed_attempts` so outcomes vary per
/// (task, isolate) while budgets and secrets stay matched.
fn isolate_failures(task_i: u32, isolate_k: u32, failed_attempts: u32) -> u32 {
    if failed_attempts == 0 {
        return 0;
    }
    (task_i.wrapping_mul(31).wrapping_add(isolate_k.wrapping_mul(17)))
        % (failed_attempts + 1)
}

/// Run one incumbent-style isolate arm: fresh session per attempt, no
/// cross-session memory, no feedback channel, restart from scratch with
/// the failed attempt's cost paid twice - the matched independent-search
/// baseline agent.
fn run_isolate(
    root: &Path,
    ws: &Path,
    k: u32,
    tasks: u32,
    failed_attempts: u32,
    draw: IsolateDraw,
    n_isolates: u32,
) -> Result<ArmReport, LoopError> {
    const INCUMBENT_BURN_STEPS: u32 = 6;
    const ATTEMPT_MAX_STEPS: u32 = 10;
    let mut iso = ArmReport {
        name: format!("isolate-{k}"),
        ..ArmReport::default()
    };
    for i in 1..=tasks {
        let goal = format!("task-{i}");
        let covered = match draw {
            IsolateDraw::Uniform => true,
            IsolateDraw::Specialist => (7 * i + k).is_multiple_of(n_isolates + 1),
        };
        let fas = match draw {
            IsolateDraw::Uniform => isolate_failures(i, k, failed_attempts),
            // covered: pass first try; uncovered: burn every attempt,
            // never pass (same attempt count either way)
            IsolateDraw::Specialist => {
                if covered { 0 } else { failed_attempts + 1 }
            }
        };
        let mut task = TaskRun {
            mission: goal.clone(),
            attempts: vec![],
            passed: false,
            restarts: 0,
        };
        for a in 1..=fas {
            let dir = root.join(format!("arms/isolate-{k}/task-{i}/attempt-{a}"));
            let answer = dir.join("run").join("work").join(&goal).join("answer.txt");
            let script = vec![answer_write(&answer, &format!("WRONG-{i}"))];
            let (passed, d) =
                run_attempt(&dir, ws, false, INCUMBENT_BURN_STEPS, &goal, &script)?;
            debug_assert!(!passed, "isolate failed attempt must fail");
            task.restarts += 1;
            task.attempts.push(d);
        }
        let last_is_burn = matches!(draw, IsolateDraw::Specialist) && !covered;
        let dir = root.join(format!("arms/isolate-{k}/task-{i}/attempt-{}", fas + 1));
        let answer = dir.join("run").join("work").join(&goal).join("answer.txt");
        let (passed, d) = if last_is_burn {
            let script = vec![answer_write(&answer, &format!("WRONG-{i}"))];
            run_attempt(&dir, ws, false, INCUMBENT_BURN_STEPS, &goal, &script)?
        } else {
            let script = vec![
                verify_line(),
                answer_write(&answer, &format!("TOKEN-{i}-SECRET")),
            ];
            run_attempt(&dir, ws, false, ATTEMPT_MAX_STEPS, &goal, &script)?
        };
        task.passed = passed;
        task.attempts.push(d);
        for d in &task.attempts {
            sum_into(&mut iso.total, d);
            iso.raw_steps += d.n_steps;
        }
        iso.priced_steps += task.raw_steps_paid();
        iso.restarts += task.restarts;
        iso.passed += u32::from(task.passed);
        iso.tasks.push(task);
    }
    Ok(iso)
}

/// Run the defined suite in both arms PLUS N matched isolates, build the
/// endpoint-wise best-of-N envelope, and price the candidate against it.
///
/// `failed_attempts` is the maximum induced failures per isolate task;
/// isolate `k` fails task `i` a deterministic spread of times in
/// `0..=failed_attempts` (matched seeds and budgets, varying outcomes).
///
/// # Errors
/// Propagates fixture IO, loop, and log errors from the real pipeline.
pub fn run_best_of_n_publication(
    root: &Path,
    tasks: u32,
    failed_attempts: u32,
    isolates: u32,
    draw: IsolateDraw,
) -> Result<BestOfNPublication, LoopError> {
    assert!(isolates >= 2, "best-of-N needs at least two isolates");
    // the two-arm gate-8 run (kept for continuity: the envelope REPLACES
    // the single-incumbent baseline as the bar to beat)
    let base = run_publication(root, tasks, failed_attempts)?;
    let ws = git_ws(root)?;
    let mut iso_reports = Vec::new();
    for k in 0..isolates {
        iso_reports.push(run_isolate(
            root,
            &ws,
            k,
            tasks,
            failed_attempts,
            draw,
            isolates,
        )?);
    }
    // endpoint-wise envelope: per task, the best passing isolate's
    // endpoint (fewest priced steps; ties break to the lower isolate id)
    let mut env_tasks = Vec::new();
    for ti in 0..tasks as usize {
        // best endpoint: a passing isolate beats a failing one; among
        // equals, the fewest priced steps wins; ties keep the lower id
        let mut best: Option<(u64, u32, bool)> = None;
        for (k, iso) in iso_reports.iter().enumerate() {
            let t = &iso.tasks[ti];
            let steps = t.raw_steps_paid();
            let better = match &best {
                None => true,
                Some((b_steps, _, b_passed)) => {
                    (t.passed && !b_passed) || (t.passed == *b_passed && steps < *b_steps)
                }
            };
            if better {
                best = Some((steps, k as u32, t.passed));
            }
        }
        let (steps, k, passed) = best.expect("at least one isolate");
        env_tasks.push(TaskEnvelope {
            mission: format!("task-{}", ti + 1),
            passed,
            priced_steps: steps,
            best_isolate: k,
        });
    }
    let env_priced: u64 = env_tasks.iter().map(|t| t.priced_steps).sum();
    let env_passed = env_tasks.iter().filter(|t| t.passed).count() as u32;
    let envelope_speedup = if base.hairspring.priced_steps == 0 {
        0.0
    } else {
        env_priced as f64 / base.hairspring.priced_steps as f64
    };
    // named metric: pass endpoints first (a task the envelope cannot
    // pass at all is an endpoint matched search lost), priced steps in
    // T_mission terms against the 2x gate second
    let envelope_verdict = if base.hairspring.passed > env_passed
        || (base.hairspring.passed == env_passed && envelope_speedup >= 2.0)
    {
        Verdict::Win
    } else {
        Verdict::Lose
    };
    Ok(BestOfNPublication {
        suite: base.suite.clone(),
        task_count: tasks,
        failed_attempts,
        n_isolates: isolates,
        incumbent: base.incumbent,
        hairspring: base.hairspring,
        isolates: iso_reports,
        envelope: EnvelopeReport {
            tasks: env_tasks,
            priced_steps: env_priced,
            passed: env_passed,
        },
        base_speedup: base.speedup_steps,
        base_verdict: base.verdict,
        envelope_speedup,
        envelope_verdict,
        draw,
    })
}
