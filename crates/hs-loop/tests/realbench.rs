//! REAL-MODEL ablation re-run of gate 3 (parent-approved spend, hard cap
//! $2 total across both models, enforced via a shared ledger file). These
//! tests are #[ignore]d: they run only on explicit request with real keys
//! populated via vault (HS_GLM_API_KEY[_FILE], HS_DEEPSEEK_API_KEY[_FILE]).
//!
//!   cargo test -p hs-loop --test realbench -- --ignored --nocapture
//!
//! Measurement, not a pass/fail gate: publishes both arms' steps-to-pass,
//! pass rate, and actual cost per model. A ledger total past the cap fails
//! the run and stops further calls.

use hs_loop::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const GLM_BIN: &str = env!("CARGO_BIN_EXE_hs-plugin-glm");
const DEEPSEEK_BIN: &str = env!("CARGO_BIN_EXE_hs-plugin-deepseek");

const TASKS: usize = 24;
const MAX_STEPS: u32 = 6;

fn ledger_path() -> std::path::PathBuf {
    std::env::var("HS_REALBENCH_LEDGER")
        .map(Into::into)
        .unwrap_or_else(|_| "/tmp/hs-realbench-ledger.txt".into())
}

fn cap_micros() -> u64 {
    std::env::var("HS_REALBENCH_CAP_MICROS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2_000_000)
}

fn ledger_total() -> u64 {
    std::fs::read_to_string(ledger_path())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.parse::<u64>().ok())
        .sum()
}

fn progress_path() -> std::path::PathBuf {
    std::env::var("HS_REALBENCH_PROGRESS")
        .map(Into::into)
        .unwrap_or_else(|_| "/tmp/hs-realbench-progress.txt".into())
}

/// Prior completed missions for this (model, arm): tag -> (passed, steps, cost).
fn progress_done(
    model: &str,
    feedback: bool,
) -> std::collections::HashMap<String, (bool, u64, u64)> {
    let prefix = format!("{model}:{feedback}:");
    std::fs::read_to_string(progress_path())
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with(&prefix))
        .filter_map(|l| {
            let mut p = l.split(':');
            let tag = p.nth(2)?.to_string();
            Some((
                tag,
                (
                    p.next()? == "1",
                    p.next()?.parse().ok()?,
                    p.next()?.parse().ok()?,
                ),
            ))
        })
        .collect()
}

fn progress_add(model: &str, feedback: bool, task: usize, passed: bool, steps: u32, cost: u64) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(progress_path())
        .unwrap();
    writeln!(
        f,
        "{model}:{feedback}:task-{task}:{}:{steps}:{cost}",
        passed as u8
    )
    .unwrap();
}

fn ledger_add(micros: u64) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(ledger_path())
        .unwrap();
    writeln!(f, "{micros}").unwrap();
}

struct Arm {
    passed: usize,
    steps_total: u64,
    cost_micros: u64,
    stopped_early: bool,
}

fn run_arm(root: &std::path::Path, feedback: bool, model_name: &str, model_bin: &str) -> Arm {
    let mut arm = Arm {
        passed: 0,
        steps_total: 0,
        cost_micros: 0,
        stopped_early: false,
    };
    for t in 0..TASKS {
        if ledger_total() >= cap_micros() {
            eprintln!("REALBENCH BUDGET CAP reached before arm={feedback} task-{t}; stopping");
            arm.stopped_early = true;
            break;
        }
        let dir = root.join(format!("{model_name}-arm-{feedback}-task-{t}"));
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("hairspring.toml");
        std::fs::write(
            &config,
            format!(
                r#"
[[tools]]
name = "answer.write"
command = ["{ANSWER}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{CHECKER}"]
subjects = ["*"]

[[models]]
name = "{model_name}"
command = ["{model_bin}"]
default = true
"#
            ),
        )
        .unwrap();
        let kernel = hs_kernel::Kernel::load(&config).unwrap();
        let log = dir.join("log");
        let mut l = InnerLoop::new(kernel, &log, feedback, MAX_STEPS).unwrap();
        let r = match l.run_mission(&format!("task-{t}")) {
            Ok(r) => r,
            Err(e) => {
                panic!("mission task-{t} errored (not a wrong answer, a harness error): {e:?}")
            }
        };
        hs_log::verify_stream(&log, r.stream_id).unwrap();
        let cost = l.total_cost_micros();
        ledger_add(cost);
        arm.cost_micros += cost;
        progress_add(model_name, feedback, t, r.passed, r.steps, cost);
        if r.passed {
            arm.passed += 1;
        }
        arm.steps_total += r.steps as u64;
    }
    arm
}

fn report(model: &str, on: &Arm, off: &Arm) {
    let on_steps = on.steps_total as f64 / TASKS as f64;
    let off_steps = off.steps_total as f64 / TASKS as f64;
    println!("REALBENCH {model}: feedback ablation vs scripted-bench ground truth");
    println!(
        "  steps-to-pass (mean, failures at cap {MAX_STEPS}): ON {on_steps:.2} vs OFF {off_steps:.2}"
    );
    println!(
        "  pass rate: ON {}/{} vs OFF {}/{}",
        on.passed, TASKS, off.passed, TASKS
    );
    println!(
        "  actual cost: ON ${:.4} + OFF ${:.4} = ${:.4} (ledger total ${:.4} of ${:.2} cap)",
        on.cost_micros as f64 / 1e6,
        off.cost_micros as f64 / 1e6,
        (on.cost_micros + off.cost_micros) as f64 / 1e6,
        ledger_total() as f64 / 1e6,
        cap_micros() as f64 / 1e6
    );
    assert!(
        ledger_total() <= cap_micros(),
        "REALBENCH BUDGET CAP EXCEEDED: {} micros > {}",
        ledger_total(),
        cap_micros()
    );
    assert!(
        !on.stopped_early && !off.stopped_early,
        "run stopped early on budget"
    );
}

#[test]
#[ignore = "real API spend; run explicitly with vault-populated keys"]
fn real_ablation_glm() {
    let root = tempfile::tempdir().unwrap();
    let on = run_arm(root.path(), true, "glm", GLM_BIN);
    let off = run_arm(root.path(), false, "glm", GLM_BIN);
    report("glm-5.3", &on, &off);
}

#[test]
#[ignore = "real API spend; run explicitly with vault-populated keys"]
fn real_ablation_deepseek() {
    let root = tempfile::tempdir().unwrap();
    let label = std::env::var("HS_DEEPSEEK_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    let on = run_arm(root.path(), true, "deepseek", DEEPSEEK_BIN);
    let off = run_arm(root.path(), false, "deepseek", DEEPSEEK_BIN);
    report(&label, &on, &off);
}
