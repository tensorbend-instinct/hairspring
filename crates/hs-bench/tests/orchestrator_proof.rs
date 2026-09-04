//! GATE 8 BENCHMARK PREP 4 - run orchestrator with run-level spend cap.
//!
//! The orchestrator runs a set of instances on one arm, enforcing: per-mission
//! cap (kills land as BudgetKilled) and a RUN-level cap that stops launching
//! new missions when the run's budget is spent - remaining instances are
//! marked NotRun, never silently dropped. Falsifiable: if the run can exceed
//! its cap, the spend gate Eric approved is broken.

use hs_bench::*;
use std::path::Path;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/mini_swebench.jsonl");

/// Scripted executor: every mission costs `cost` micro-USD and resolves
/// (stands in for the real hs-loop mission; the loop's own budget kill is
/// proven in hs-loop/tests/budget.rs).
struct ScriptedExec {
    cost: u64,
}
impl MissionExec for ScriptedExec {
    fn run_mission(
        &self,
        inst: &BenchInstance,
        arm: Arm,
        _mission_cap: u64,
    ) -> Result<InstanceResult, BenchError> {
        Ok(InstanceResult {
            instance_id: inst.instance_id.clone(),
            arm,
            outcome: Outcome::Resolved,
            cost_micros: self.cost,
        })
    }
}

#[test]
fn run_cap_stops_launching_and_marks_not_run() {
    let instances = load_jsonl(Path::new(FIXTURE)).unwrap(); // 3 instances
    let exec = ScriptedExec { cost: 400_000 }; // $0.40 per mission
    let report = run_set(&exec, &instances, Arm::System, 900_000, 1_000_000);
    // $0.90 run cap: two $0.40 missions fit, the third must be NotRun
    assert_eq!(report.resolved_count(Arm::System), 2);
    assert_eq!(report.not_run_count(), 1, "third instance must be NotRun");
    assert!(report.total_cost_micros() <= 900_000);
    let json = report.to_swebench_json("test");
    assert_eq!(json["not_run"].as_array().unwrap().len(), 1);
}

#[test]
fn uncapped_run_completes_everything() {
    let instances = load_jsonl(Path::new(FIXTURE)).unwrap();
    let exec = ScriptedExec { cost: 100 };
    let report = run_set(&exec, &instances, Arm::System, 1_000_000_000, 1_000_000);
    assert_eq!(report.resolved_count(Arm::System), 3);
    assert_eq!(report.not_run_count(), 0);
}
