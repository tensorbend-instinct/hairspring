//! GATE 8 BENCHMARK PREP 5 - mission prompt builder (offline).
//!
//! The real benchmark mission prompt must carry: the problem statement, the
//! workspace layout, and a response contract the patch extractor can parse
//! back. Round-trip proof: a completion that follows the contract extracts
//! to an applicable patch.

use hs_bench::*;
use std::path::Path;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/mini_swebench.jsonl");

#[test]
fn prompt_carries_statement_layout_and_contract() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(ws.join("src")).unwrap();
    std::fs::write(ws.join("code.txt"), "broken\n").unwrap();
    std::fs::write(ws.join("src/lib.rs"), "fn f() {}\n").unwrap();

    let instances = load_jsonl(Path::new(FIXTURE)).unwrap();
    let prompt = mission_prompt(&instances[0], &ws);
    assert!(prompt.contains("code.txt must contain the word fixed"));
    assert!(prompt.contains("code.txt"), "layout must list files");
    assert!(prompt.contains("src/lib.rs"));
    assert!(prompt.contains("```diff"), "contract must demand a fenced diff");
    assert!(prompt.contains(&instances[0].instance_id));
}

#[test]
fn contract_round_trips_through_extractor() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("code.txt"), "broken\n").unwrap();
    let instances = load_jsonl(Path::new(FIXTURE)).unwrap();
    let _prompt = mission_prompt(&instances[0], &ws);
    // a model that follows the contract answers with a fenced diff
    let completion = format!("Analysis: the file needs the fixed token.\n```diff\n{}\n```", instances[0].patch.trim());
    let patch = extract_patch(&completion).expect("contract-following completion must extract");
    let applied = apply_model_patch(&ws, &patch).unwrap();
    assert!(matches!(applied, ApplyResult::Applied));
    assert_eq!(std::fs::read_to_string(ws.join("code.txt")).unwrap(), "fixed\n");
}
