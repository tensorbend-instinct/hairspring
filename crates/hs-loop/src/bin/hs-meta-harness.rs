//! Filesystem-native Meta-Harness CLI.
//!
//! Proposer arguments: `ITERATION HISTORY_ROOT OUTPUT_JSON`.
//! Evaluator arguments: `CANDIDATE_SOURCE_ROOT TASK TRIAL OUTPUT_JSON`.
use hs_loop::meta_harness::{CandidateProposal, MetaHarness, SearchConfig, TrialOutcome};
use std::path::{Path, PathBuf};
fn arg(a: &[String], f: &str) -> Option<String> {
    a.iter()
        .position(|x| x == f)
        .and_then(|i| a.get(i + 1))
        .cloned()
}
fn fail(m: &str) -> ! {
    eprintln!("hs-meta-harness: {m}");
    std::process::exit(2)
}
fn json<T: serde::de::DeserializeOwned>(p: &Path) -> T {
    serde_json::from_slice(
        &std::fs::read(p).unwrap_or_else(|e| fail(&format!("{}: {e}", p.display()))),
    )
    .unwrap_or_else(|e| fail(&format!("{}: {e}", p.display())))
}
fn exec(p: &str, a: &[String]) {
    let s = std::process::Command::new(p)
        .args(a)
        .status()
        .unwrap_or_else(|e| fail(&format!("start {p}: {e}")));
    if !s.success() {
        fail(&format!("{p} exited {s}"))
    }
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.get(1).map(String::as_str) != Some("run") {
        fail(
            "usage: hs-meta-harness run --root DIR --iterations N --trials N --tasks FILE --baseline NAME --proposer PROGRAM --evaluator PROGRAM",
        )
    }
    let root = PathBuf::from(arg(&a, "--root").unwrap_or_else(|| fail("--root required")));
    let iterations = arg(&a, "--iterations")
        .unwrap_or_else(|| fail("--iterations required"))
        .parse()
        .unwrap_or_else(|_| fail("--iterations must be a number"));
    let trials = arg(&a, "--trials")
        .unwrap_or_else(|| fail("--trials required"))
        .parse()
        .unwrap_or_else(|_| fail("--trials must be a number"));
    let tp = PathBuf::from(arg(&a, "--tasks").unwrap_or_else(|| fail("--tasks required")));
    let tasks = std::fs::read_to_string(&tp)
        .unwrap_or_else(|e| fail(&format!("{}: {e}", tp.display())))
        .lines()
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(str::to_owned)
        .collect();
    let cfg = SearchConfig {
        iterations,
        trials_per_task: trials,
        search_tasks: tasks,
        baseline_name: arg(&a, "--baseline").unwrap_or_else(|| fail("--baseline required")),
    };
    let proposer = arg(&a, "--proposer").unwrap_or_else(|| fail("--proposer required"));
    let evaluator = arg(&a, "--evaluator").unwrap_or_else(|| fail("--evaluator required"));
    let r = MetaHarness::new(&root, cfg)
        .run(
            |it, h| {
                let o = root.join(format!(".proposal-{it}.json"));
                exec(
                    &proposer,
                    &[
                        it.to_string(),
                        h.display().to_string(),
                        o.display().to_string(),
                    ],
                );
                let v = json::<CandidateProposal>(&o);
                let _ = std::fs::remove_file(o);
                v
            },
            |c, t, n| {
                let s = std::fs::read_dir(root.join("iterations"))
                    .unwrap_or_else(|e| fail(&format!("iterations: {e}")))
                    .filter_map(Result::ok)
                    .map(|entry| entry.path().join("candidates").join(&c.name).join("source"))
                    .filter(|path| path.exists())
                    .max()
                    .unwrap_or_else(|| fail(&format!("source missing for {}", c.name)));
                let o = root.join(format!(".eval-{}-{t}-{n}.json", c.name));
                exec(
                    &evaluator,
                    &[
                        s.display().to_string(),
                        t.to_string(),
                        n.to_string(),
                        o.display().to_string(),
                    ],
                );
                let v = json::<TrialOutcome>(&o);
                let _ = std::fs::remove_file(o);
                v
            },
        )
        .unwrap_or_else(|e| fail(&e.to_string()));
    let completed = std::fs::read_to_string(root.join("evolution_summary.jsonl"))
        .unwrap_or_default()
        .lines()
        .count();
    println!(
        "{}",
        serde_json::json!({"frontier":r.frontier,"iterations_completed":completed,"history_root":r.root})
    );
}
