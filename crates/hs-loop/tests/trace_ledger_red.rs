#[test]
fn scratch_shell_execution_is_verification_but_refused_edit_is_not() {
    let mut l = hs_loop::ledger::Ledger::default();
    l.apply_tool_call(1, "repo.exec", &serde_json::json!({"command":"git apply x.diff"}), &serde_json::json!({"scratch":true,"applied":false,"exit_code":-1,"error":"forbidden edit"}));
    assert!(!l.model_verified());
    l.apply_tool_call(2, "repo.exec", &serde_json::json!({"command":"test -f code.txt"}), &serde_json::json!({"scratch":true,"applied":false,"exit_code":0,"stdout":"","stderr":""}));
    assert!(l.model_verified(), "completed scratch-shell checks must enter the evidence ledger");
}
