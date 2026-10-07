//! H2H t1 (2026-10-06): a read-only question took 183s because the verifier saw
//! only "export.rs:L1-400" in the ledger, refuted "unverifiable" (no recorded
//! content), and cost a 50s round plus two more submits. Read evidence must
//! carry a bounded, marked excerpt of what was actually read.

use serde_json::json;

#[test]
fn repo_read_records_bounded_content_evidence() {
    let mut l = hs_loop::ledger::Ledger::default();
    let body = format!("//! Export a session as ZIP\nfn export_zip() {{ /* writes stored entries */ }}\n{}\n// END_MARKER", "x".repeat(3000));
    l.apply_tool_call(1, "repo.read", &json!({"path": "src/export.rs"}),
        &json!({"content": body, "total_lines": 80}));
    let s = l.summary();
    assert!(s.contains("export_zip"), "head excerpt of the read is in the ledger: {s}");
    assert!(s.contains("END_MARKER"), "tail excerpt of the read is in the ledger: {s}");
    assert!(s.contains("chars]"), "truncation is marked, never silent: {s}");
    assert!(s.len() < 4000, "bounded: {}", s.len());
}
