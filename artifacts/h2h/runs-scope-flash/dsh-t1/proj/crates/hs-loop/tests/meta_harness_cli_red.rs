//! User-path gate: the shipped CLI runs a proposer and external evaluator,
//! persists the complete filesystem history, resumes, and prints frontier JSON.

use std::process::Command;

#[cfg(unix)]
fn executable(path: &std::path::Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, body).unwrap();
    let mut p = std::fs::metadata(path).unwrap().permissions();
    p.set_mode(0o755);
    std::fs::set_permissions(path, p).unwrap();
}

#[test]
#[cfg(unix)]
fn cli_runs_normal_proposer_evaluator_path_and_resumes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("history");
    let proposer = tmp.path().join("proposer.py");
    let evaluator = tmp.path().join("evaluator.py");
    let tasks = tmp.path().join("tasks.txt");
    std::fs::write(&tasks, "task-a\n").unwrap();
    executable(
        &proposer,
        r"#!/usr/bin/env python3
import json,sys,pathlib
it=int(sys.argv[1]); root=pathlib.Path(sys.argv[2]); out=sys.argv[3]
if it == 2:
  assert 'failure-marker' in (root/'iterations/0001/candidates/cand-1/trials/task-a/0001/trace.log').read_text()
json.dump({'name':f'cand-{it}','parent':'baseline' if it==1 else 'cand-1','hypothesis':'read full history','reflection':'diagnosed raw trace','files':{'harness.txt':f'candidate {it}'}},open(out,'w'))
",
    );
    executable(
        &evaluator,
        r"#!/usr/bin/env python3
import json,sys,pathlib
candidate=pathlib.Path(sys.argv[1]).parent.name; task=sys.argv[2]; trial=int(sys.argv[3]); out=sys.argv[4]
passed=candidate=='cand-2'
json.dump({'task':task,'trial':trial,'passed':passed,'score':1.0 if passed else 0.0,'trace':'success-marker' if passed else 'failure-marker','error':None},open(out,'w'))
",
    );
    let bin = env!("CARGO_BIN_EXE_hs-meta-harness");
    let output = Command::new(bin)
        .args([
            "run",
            "--root",
            root.to_str().unwrap(),
            "--iterations",
            "2",
            "--trials",
            "2",
            "--tasks",
            tasks.to_str().unwrap(),
            "--baseline",
            "baseline",
            "--proposer",
            proposer.to_str().unwrap(),
            "--evaluator",
            evaluator.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(v["frontier"]["name"], "cand-2");
    assert_eq!(v["iterations_completed"], 2);
    let output2 = Command::new(bin)
        .args([
            "run",
            "--root",
            root.to_str().unwrap(),
            "--iterations",
            "1",
            "--trials",
            "2",
            "--tasks",
            tasks.to_str().unwrap(),
            "--baseline",
            "baseline",
            "--proposer",
            proposer.to_str().unwrap(),
            "--evaluator",
            evaluator.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output2.status.success(),
        "{}",
        String::from_utf8_lossy(&output2.stderr)
    );
    assert!(root.join("iterations/0003").exists());
    assert_eq!(
        std::fs::read_to_string(root.join("evolution_summary.jsonl"))
            .unwrap()
            .lines()
            .count(),
        3
    );
}
