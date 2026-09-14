//! RED-first acceptance for Lee et al. Meta-Harness.
//!
//! The optimizer must expose the complete prior population as a filesystem:
//! candidate source, raw trial traces, scores, and reflections. Selection is
//! made from external trial outcomes, never the proposer's account.

use hs_loop::meta_harness::{CandidateProposal, MetaHarness, SearchConfig, TrialOutcome};
use std::cell::RefCell;
use std::rc::Rc;

#[test]
fn full_history_filesystem_drives_later_proposals_and_external_scores_choose_frontier() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("evolution");
    let seen_second = Rc::new(RefCell::new(false));
    let seen = Rc::clone(&seen_second);
    let proposer = move |iteration: u32, history: &std::path::Path| {
        if iteration == 2 {
            let source = std::fs::read_to_string(
                history.join("iterations/0001/candidates/cand-1/source/harness.rs"),
            )
            .unwrap();
            let trace = std::fs::read_to_string(
                history.join("iterations/0001/candidates/cand-1/trials/task-a/0001/trace.log"),
            )
            .unwrap();
            let score = std::fs::read_to_string(
                history.join("iterations/0001/candidates/cand-1/score.json"),
            )
            .unwrap();
            assert!(source.contains("bad_context"));
            assert!(trace.contains("context truncated before error"));
            assert!(score.contains("0.0"));
            *seen.borrow_mut() = true;
        }
        CandidateProposal {
            name: format!("cand-{iteration}"),
            parent: if iteration == 1 {
                "baseline".into()
            } else {
                "cand-1".into()
            },
            hypothesis: if iteration == 1 {
                "try context compaction".into()
            } else {
                "preserve the failing tail".into()
            },
            reflection: format!("iteration {iteration} diagnosis from raw traces"),
            files: [(
                "harness.rs".into(),
                if iteration == 1 {
                    "fn bad_context() {}".into()
                } else {
                    "fn preserve_tail() {}".into()
                },
            )]
            .into(),
        }
    };
    let evaluator = |candidate: &CandidateProposal, task: &str, trial: u32| {
        let passed = candidate.name == "cand-2";
        TrialOutcome {
            task: task.into(),
            trial,
            passed,
            score: if passed { 1.0 } else { 0.0 },
            trace: if passed {
                "kept failing tail; verifier passed".into()
            } else {
                "context truncated before error".into()
            },
            error: None,
        }
    };
    let cfg = SearchConfig {
        iterations: 2,
        trials_per_task: 2,
        search_tasks: vec!["task-a".into()],
        baseline_name: "baseline".into(),
    };
    let result = MetaHarness::new(&root, cfg)
        .run(proposer, evaluator)
        .unwrap();
    assert!(
        *seen_second.borrow(),
        "iteration 2 read iteration 1's raw source, trace, and score"
    );
    assert_eq!(result.frontier.name, "cand-2");
    assert_eq!(result.frontier.mean_score, 1.0);
    assert!(
        std::fs::read_to_string(root.join("frontier.json"))
            .unwrap()
            .contains("cand-2")
    );
    assert_eq!(
        std::fs::read_to_string(root.join("evolution_summary.jsonl"))
            .unwrap()
            .lines()
            .count(),
        2
    );
}

#[test]
fn corrupt_or_missing_trial_evidence_fails_closed_and_resume_never_erases_history() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("evolution");
    let cfg = SearchConfig {
        iterations: 1,
        trials_per_task: 2,
        search_tasks: vec!["task-a".into()],
        baseline_name: "baseline".into(),
    };
    let proposer = |iteration, _history: &std::path::Path| CandidateProposal {
        name: format!("cand-{iteration}"),
        parent: "baseline".into(),
        hypothesis: "x".into(),
        reflection: "r".into(),
        files: [("harness.rs".into(), "fn harness() {}".into())].into(),
    };
    let result = MetaHarness::new(&root, cfg.clone())
        .run(proposer, |_c, task, trial| TrialOutcome {
            task: task.into(),
            trial,
            passed: trial == 1,
            score: if trial == 1 { 1.0 } else { f64::NAN },
            trace: if trial == 1 {
                "complete".into()
            } else {
                String::new()
            },
            error: None,
        })
        .unwrap();
    assert_eq!(
        result.frontier.mean_score, 0.5,
        "invalid evidence is a zero, not omitted"
    );
    let first = std::fs::read_to_string(
        root.join("iterations/0001/candidates/cand-1/trials/task-a/0001/trace.log"),
    )
    .unwrap();

    let cfg2 = SearchConfig {
        iterations: 1,
        ..cfg
    };
    MetaHarness::new(&root, cfg2)
        .run(proposer, |_c, task, trial| TrialOutcome {
            task: task.into(),
            trial,
            passed: true,
            score: 1.0,
            trace: "second iteration".into(),
            error: None,
        })
        .unwrap();
    assert_eq!(
        first,
        std::fs::read_to_string(
            root.join("iterations/0001/candidates/cand-1/trials/task-a/0001/trace.log")
        )
        .unwrap()
    );
    assert!(
        root.join("iterations/0002/candidates/cand-2/source/harness.rs")
            .exists()
    );
}

#[test]
fn candidate_and_task_paths_cannot_escape_history_root() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("history");
    let cfg = SearchConfig {
        iterations: 1,
        trials_per_task: 1,
        search_tasks: vec!["../escape".into()],
        baseline_name: "baseline".into(),
    };
    let result = MetaHarness::new(&root, cfg).run(
        |_, _| CandidateProposal {
            name: "../../candidate".into(),
            parent: "baseline".into(),
            hypothesis: "x".into(),
            reflection: "x".into(),
            files: [("../../outside".into(), "bad".into())].into(),
        },
        |_c, task, trial| TrialOutcome {
            task: task.into(),
            trial,
            passed: true,
            score: 1.0,
            trace: "x".into(),
            error: None,
        },
    );
    assert!(result.is_err());
    assert!(!tmp.path().join("outside").exists());
}

#[test]
fn resume_rejects_a_different_search_protocol() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("history");
    let cfg = SearchConfig {
        iterations: 1,
        trials_per_task: 1,
        search_tasks: vec!["task-a".into()],
        baseline_name: "baseline".into(),
    };
    let proposer = |iteration, _history: &std::path::Path| CandidateProposal {
        name: format!("cand-{iteration}"),
        parent: "baseline".into(),
        hypothesis: "x".into(),
        reflection: "x".into(),
        files: [("harness".into(), "x".into())].into(),
    };
    MetaHarness::new(&root, cfg)
        .run(proposer, |_c, task, trial| TrialOutcome {
            task: task.into(),
            trial,
            passed: true,
            score: 1.0,
            trace: "x".into(),
            error: None,
        })
        .unwrap();
    let changed = SearchConfig {
        iterations: 1,
        trials_per_task: 2,
        search_tasks: vec!["task-b".into()],
        baseline_name: "other".into(),
    };
    let result = MetaHarness::new(&root, changed).run(proposer, |_c, task, trial| TrialOutcome {
        task: task.into(),
        trial,
        passed: true,
        score: 1.0,
        trace: "x".into(),
        error: None,
    });
    assert!(
        result.is_err(),
        "a resumed search cannot silently change its baseline, tasks, or trial count"
    );
    assert!(!root.join("iterations/0002").exists());
}
