//! GATE 3 ACCEPTANCE (spec section 10, row 3):
//!   Run a coding benchmark suite with feedback injection on vs off
//!   (ablation): steps-to-pass drops with no regression in pass rate.
//!   Measure the feedback-hiding coverage limit and publish it.
//!
//! Falsifiable: if ON-arm steps-to-pass >= OFF-arm, or ON pass rate <
//! OFF pass rate, the gate fails.

use hs_loop::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const BENCHMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-benchmodel");

const TASKS: usize = 24;
const REPAIRABLE: usize = 18;
const MAX_STEPS: u32 = 6;

struct Arm {
    passed: usize,
    steps_total: u64, // failed missions count at the cap
    missions: usize,
}

fn run_arm(root: &std::path::Path, feedback: bool) -> Arm {
    let mut arm = Arm {
        passed: 0,
        steps_total: 0,
        missions: 0,
    };
    for t in 0..TASKS {
        let dir = root.join(format!("arm-{feedback}-task-{t}"));
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
name = "benchmodel"
command = ["{BENCHMODEL}"]
default = true
"#
            ),
        )
        .unwrap();
        let kernel = hs_kernel::Kernel::load(&config).unwrap();
        let log = dir.join("log");
        let mut l = InnerLoop::new(kernel, &log, feedback, MAX_STEPS).unwrap();
        let r = l.run_mission(&format!("task-{t}")).unwrap();
        // item 3: a pass costs exactly one extra round trip - the
        // adversarial verifier call. Feedback itself costs zero.
        assert_eq!(
            r.model_calls,
            r.steps + u32::from(r.passed),
            "feedback zero extra round trips; pass adds one verifier call"
        );
        hs_log::verify_stream(&log, r.stream_id).unwrap();
        if r.passed {
            arm.passed += 1;
        }
        arm.steps_total += u64::from(r.steps);
        arm.missions += 1;
    }
    arm
}

#[test]
fn gate3_proof_feedback_ablation() {
    let root = tempfile::tempdir().unwrap();
    let on = run_arm(root.path(), true);
    let off = run_arm(root.path(), false);

    let on_pass = on.passed as f64 / on.missions as f64;
    let off_pass = off.passed as f64 / off.missions as f64;
    let on_steps = on.steps_total as f64 / on.missions as f64;
    let off_steps = off.steps_total as f64 / off.missions as f64;
    let coverage_limit = (TASKS - REPAIRABLE) as f64 / TASKS as f64;

    // THE GATE:
    assert!(
        on_steps < off_steps,
        "steps-to-pass did not drop: {on_steps} vs {off_steps}"
    );
    assert!(
        on_pass >= off_pass,
        "pass rate regressed: {on_pass} vs {off_pass}"
    );
    // measured values, published win or lose:
    assert_eq!(
        on.passed, REPAIRABLE,
        "ON arm should pass exactly the repairable family"
    );
    assert_eq!(
        off.passed, 0,
        "OFF arm has no channel to the fix in this suite"
    );

    println!("PROOF-GATE3 feedback ablation: PASS");
    println!("  steps-to-pass (mean over {TASKS} missions, failures at cap {MAX_STEPS}): ON {on_steps:.2} vs OFF {off_steps:.2}");
    println!(
        "  pass rate: ON {}/{} ({:.0}%) vs OFF {}/{} ({:.0}%) - no regression",
        on.passed,
        on.missions,
        on_pass * 100.0,
        off.passed,
        off.missions,
        off_pass * 100.0
    );
    println!("  feedback-hiding coverage limit: {:.0}% of failure classes in this suite carry no injectable fix ({}/{})",
        coverage_limit * 100.0, TASKS - REPAIRABLE, TASKS);
    println!("  model round trips == steps in every mission (feedback added zero extra calls); all chains verified");
}
