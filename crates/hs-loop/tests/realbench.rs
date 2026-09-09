//! REAL-MODEL ablation re-run of gate 3 (parent-approved spend, hard cap
//! $6 total across both models - Eric 2026-09-03 00:03; enforced via a
//! shared ledger file; actual final spend $1.5644). These
//! tests are #[ignore]d: they run only on explicit request with real keys
//! populated via vault (`HS_GLM_API_KEY`[_FILE], `HS_DEEPSEEK_API_KEY`[_FILE]).
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
    std::env::var("HS_REALBENCH_LEDGER").map_or_else(|_| "/tmp/hs-realbench-ledger.txt".into(), Into::into)
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
    std::env::var("HS_REALBENCH_PROGRESS").map_or_else(|_| "/tmp/hs-realbench-progress.txt".into(), Into::into)
}

/// Parse `HS_REALBENCH_TASKS`: comma list with optional a-b ranges ("0-5,12,18-23").
/// Default: all TASKS.
fn tasks_from_env() -> Vec<usize> {
    let Ok(v) = std::env::var("HS_REALBENCH_TASKS") else {
        return (0..TASKS).collect();
    };
    let mut out = vec![];
    for part in v.split(',') {
        let part = part.trim();
        if let Some((a, b)) = part.split_once('-') {
            let (a, b): (usize, usize) = (a.trim().parse().unwrap(), b.trim().parse().unwrap());
            out.extend(a..=b);
        } else {
            out.push(part.parse().unwrap());
        }
    }
    out.retain(|&t| t < TASKS);
    out.sort_unstable();
    out.dedup();
    out
}

/// Optional persistent root for mission streams (`HS_REALBENCH_STREAMS`).
/// When set, per-mission logs survive process exit for the audit trail.
fn streams_root() -> Option<std::path::PathBuf> {
    std::env::var("HS_REALBENCH_STREAMS").map(Into::into).ok()
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
    // one write syscall: O_APPEND + a single write_all keeps lines atomic
    // across concurrent shard processes
    let line = format!(
        "{model}:{feedback}:task-{task}:{}:{steps}:{cost}\n",
        u8::from(passed)
    );
    f.write_all(line.as_bytes()).unwrap();
}

fn ledger_add(micros: u64) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(ledger_path())
        .unwrap();
    let line = format!("{micros}\n");
    f.write_all(line.as_bytes()).unwrap();
}

struct Arm {
    passed: usize,
    steps_total: u64,
    cost_micros: u64,
    stopped_early: bool,
}

fn run_arm(
    root: &std::path::Path,
    feedback: bool,
    model_name: &str,
    model_bin: &str,
    tasks: &[usize],
) -> Arm {
    let mut arm = Arm {
        passed: 0,
        steps_total: 0,
        cost_micros: 0,
        stopped_early: false,
    };
    let done = progress_done(model_name, feedback);
    for &t in tasks {
        let tag = format!("task-{t}");
        if let Some(&(passed, steps, cost)) = done.get(&tag) {
            eprintln!("resume: {model_name} arm={feedback} {tag} already done (passed={passed}, steps={steps})");
            if passed {
                arm.passed += 1;
            }
            arm.steps_total += steps;
            arm.cost_micros += cost;
            continue;
        }
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
        arm.steps_total += u64::from(r.steps);
    }
    arm
}

fn report(model: &str, on: &Arm, off: &Arm, n_tasks: usize) {
    let on_steps = on.steps_total as f64 / n_tasks as f64;
    let off_steps = off.steps_total as f64 / n_tasks as f64;
    println!("REALBENCH {model}: feedback ablation vs scripted-bench ground truth");
    println!(
        "  steps-to-pass (mean, failures at cap {MAX_STEPS}): ON {on_steps:.2} vs OFF {off_steps:.2}"
    );
    println!(
        "  pass rate: ON {}/{} vs OFF {}/{}",
        on.passed, n_tasks, off.passed, n_tasks
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
    let tasks = tasks_from_env();
    let keep;
    let root: &std::path::Path = if let Some(d) = streams_root() {
        std::fs::create_dir_all(&d).unwrap();
        keep = d;
        &keep
    } else {
        keep = tempfile::tempdir().unwrap().keep();
        &keep
    };
    let on = run_arm(root, true, "glm", GLM_BIN, &tasks);
    let off = run_arm(root, false, "glm", GLM_BIN, &tasks);
    report("glm-5.3", &on, &off, tasks.len());
}

#[test]
#[ignore = "real API spend; run explicitly with vault-populated keys"]
fn real_ablation_deepseek() {
    let tasks = tasks_from_env();
    let keep;
    let root: &std::path::Path = if let Some(d) = streams_root() {
        std::fs::create_dir_all(&d).unwrap();
        keep = d;
        &keep
    } else {
        keep = tempfile::tempdir().unwrap().keep();
        &keep
    };
    let label = std::env::var("HS_DEEPSEEK_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    let on = run_arm(root, true, "deepseek", DEEPSEEK_BIN, &tasks);
    let off = run_arm(root, false, "deepseek", DEEPSEEK_BIN, &tasks);
    report(&label, &on, &off, tasks.len());
}

// ---- shard/append faithfulness unit tests (no API spend) ----

// progress/ledger helpers read process-global env vars; serialize tests that touch them
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn shard_parsing_variants() {
    let _g = ENV_LOCK.lock().unwrap();
    // default: all tasks
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_REALBENCH_TASKS") };
    assert_eq!(tasks_from_env(), (0..TASKS).collect::<Vec<_>>());
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_REALBENCH_TASKS", "0-5") };
    assert_eq!(tasks_from_env(), vec![0, 1, 2, 3, 4, 5]);
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_REALBENCH_TASKS", "0-2,7,18-19") };
    assert_eq!(tasks_from_env(), vec![0, 1, 2, 7, 18, 19]);
    // out-of-range and dupes are dropped
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_REALBENCH_TASKS", "22-30,3,3") };
    assert_eq!(tasks_from_env(), vec![3, 22, 23]);
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_REALBENCH_TASKS") };
}

#[test]
fn concurrent_appends_never_interleave() {
    let _g = ENV_LOCK.lock().unwrap();
    // two shard processes appending to one progress file must produce only
    // whole parseable lines (O_APPEND + single write_all per line)
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("progress.txt");
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_REALBENCH_PROGRESS", &path) };
    let mut kids = vec![];
    for model in ["m1", "m2"] {
        let path = path.clone();
        kids.push(std::thread::spawn(move || {
            // FIXME: Audit that the environment access only happens in single-threaded code.
            unsafe { std::env::set_var("HS_REALBENCH_PROGRESS", &path) };
            for t in 0..50 {
                progress_add(model, true, t % TASKS, true, 2, 1000 + t as u64);
            }
        }));
    }
    for k in kids {
        k.join().unwrap();
    }
    let body = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines.len(), 100, "no line may be lost");
    for l in lines {
        let parts: Vec<&str> = l.split(':').collect();
        assert_eq!(parts.len(), 6, "no line may be interleaved: {l:?}");
        assert!(parts[2].starts_with("task-"));
        parts[3].parse::<u8>().unwrap();
        parts[4].parse::<u32>().unwrap();
        parts[5].parse::<u64>().unwrap();
    }
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_REALBENCH_PROGRESS") };
}

#[test]
fn progress_done_roundtrip_per_model_arm() {
    let _g = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("progress.txt");
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_REALBENCH_PROGRESS", &path) };
    progress_add("glm", true, 3, true, 2, 5000);
    progress_add("glm", false, 3, false, 6, 7000);
    progress_add("deepseek", true, 3, true, 4, 9000);
    let glm_on = progress_done("glm", true);
    assert_eq!(glm_on.get("task-3"), Some(&(true, 2, 5000)));
    let glm_off = progress_done("glm", false);
    assert_eq!(glm_off.get("task-3"), Some(&(false, 6, 7000)));
    let ds_on = progress_done("deepseek", true);
    assert_eq!(ds_on.get("task-3"), Some(&(true, 4, 9000)));
    // a shard's resume view is independent per (model, arm)
    assert!(!glm_on.contains_key("task-4"));
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_REALBENCH_PROGRESS") };
}
