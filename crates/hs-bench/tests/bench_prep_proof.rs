//! GATE 8 BENCHMARK PREP - offline plumbing proof (no paid model calls).
//!
//! The benchmark half of spec row 8 needs: a SWE-bench Verified dataset
//! loader, a mission adapter, per-mission + run-level budget enforcement
//! (kill-at-cap scores as fail), a baseline arm with feedback/self-mod
//! disabled, and a SWE-bench-shaped report. This proof pins all of it
//! offline on 3 in-repo fixture instances with scripted patch sources:
//! gold patches must resolve, wrong patches must not, a budget-burning
//! mission must be killed and scored unresolved.
//!
//! Falsifiable: if a wrong patch resolves, the eval is broken. If a mission
//! can exceed its budget without being killed, the spend gate is broken.

use hs_bench::*;
use std::path::Path;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/mini_swebench.jsonl");

#[test]
fn loader_parses_swebench_verified_jsonl() {
    let instances = load_jsonl(Path::new(FIXTURE)).unwrap();
    assert_eq!(instances.len(), 3);
    let a = &instances[0];
    assert_eq!(a.instance_id, "fixture__alpha-1");
    assert_eq!(a.repo, "fixture/alpha");
    assert_eq!(a.base_commit, "abc123");
    assert!(a.problem_statement.contains("fixed"));
    assert!(a.patch.contains("+fixed"));
    assert_eq!(a.fail_to_pass, vec!["check.sh".to_string()]);
    assert!(a.pass_to_pass.is_empty());
}

#[test]
fn offline_pipeline_gold_resolves_wrong_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let instances = load_jsonl(Path::new(FIXTURE)).unwrap();
    let runner = BenchRunner::new(tmp.path(), 1_000_000); // $1 per-mission cap

    // gold patches resolve every fixture on both arms (plumbing works)
    for arm in [Arm::Baseline, Arm::System] {
        for inst in &instances {
            let r = runner.run_fixture(inst, PatchSource::Gold, arm).unwrap();
            assert_eq!(
                r.outcome,
                Outcome::Resolved,
                "gold patch must resolve {} on {arm:?}",
                inst.instance_id
            );
        }
    }
    // wrong patch must NOT resolve - this is the eval's teeth
    let r = runner
        .run_fixture(&instances[0], PatchSource::Wrong, Arm::System)
        .unwrap();
    assert_eq!(r.outcome, Outcome::Unresolved);
}

#[test]
fn budget_burn_is_killed_and_scored_unresolved() {
    let tmp = tempfile::tempdir().unwrap();
    let instances = load_jsonl(Path::new(FIXTURE)).unwrap();
    let runner = BenchRunner::new(tmp.path(), 1_000); // 0.1 cent cap
    let r = runner
        .run_fixture(&instances[0], PatchSource::BudgetBurn, Arm::System)
        .unwrap();
    assert_eq!(
        r.outcome,
        Outcome::BudgetKilled,
        "over-cap mission must be killed"
    );
    let report = BenchReport::new(vec![r]);
    assert_eq!(
        report.resolved_count(Arm::System),
        0,
        "budget kill scores as fail"
    );
}

#[test]
fn report_is_swebench_shaped() {
    let tmp = tempfile::tempdir().unwrap();
    let instances = load_jsonl(Path::new(FIXTURE)).unwrap();
    let runner = BenchRunner::new(tmp.path(), 1_000_000);
    let mut results = vec![];
    for inst in &instances {
        results.push(
            runner
                .run_fixture(inst, PatchSource::Gold, Arm::System)
                .unwrap(),
        );
    }
    results.push(
        runner
            .run_fixture(&instances[0], PatchSource::Wrong, Arm::Baseline)
            .unwrap(),
    );
    let report = BenchReport::new(results);
    assert_eq!(report.resolved_count(Arm::System), 3);
    assert_eq!(report.resolved_count(Arm::Baseline), 0);
    let json = report.to_swebench_json("hairspring-gate8");
    // official SWE-bench report shape: per-instance resolved flags + lists
    assert!(json.get("resolved").is_some(), "missing resolved list");
    assert!(
        json.get("no_apply").is_some()
            || json.get("error").is_some()
            || json.get("unresolved").is_some()
    );
    let resolved = json["resolved"].as_array().unwrap();
    assert_eq!(resolved.len(), 3);
    assert!(resolved.iter().any(|v| v == "fixture__alpha-1"));
}
