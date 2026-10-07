//! RED: `DeepSeek` v4 thinking mode rejects `tool_choice:"required`" at the API
//! level (live 400, verified 2026-09-05: "Thinking mode does not support
//! this `tool_choice`"). The provider must carry its own `tool_choice` contract:
//! deepseek -> "auto" (the loop's no-tool-call feedback at lib.rs already
//! covers a prose reply), kimi/glm -> "required" (verified live on kimi-k3).

#[test]
fn provider_carries_tool_choice_contract() {
    assert!(
        !hs_loop::realmodel::deepseek().tool_choice_required,
        "deepseek thinking mode 400s on tool_choice:required (live-verified)"
    );
    assert!(
        hs_loop::realmodel::glm().tool_choice_required,
        "glm keeps API-enforced one-tool-call-per-reply"
    );
}

#[test]
fn build_body_honors_tool_choice_contract() {
    let tools = serde_json::json!([{"type":"function","function":{"name":"repo.search","description":"d","parameters":{"type":"object","properties":{"pattern":{"type":"string"}},"required":["pattern"]}}}]);
    let req = hs_loop::realmodel::build_body("kimi-k3", "sys", "p", Some(&tools), None, true);
    assert_eq!(req["tool_choice"], "required", "required-capable provider");
    let auto =
        hs_loop::realmodel::build_body("deepseek-v4-pro", "sys", "p", Some(&tools), None, false);
    assert_eq!(
        auto["tool_choice"], "auto",
        "thinking-mode provider falls to auto"
    );
    assert_eq!(
        auto["tools"][0]["function"]["name"], "repo__search",
        "wire mapping unchanged"
    );
}

#[test]
fn build_body_messages_honors_tool_choice_contract() {
    let tools = serde_json::json!([{"type":"function","function":{"name":"repo.search","description":"d","parameters":{"type":"object","properties":{}}}}]);
    let messages = serde_json::json!([{"role":"user","content":"MISSION: task-x"}]);
    let auto = hs_loop::realmodel::build_body_messages(
        "deepseek-v4-pro",
        "sys",
        &messages,
        Some(&tools),
        None,
        false,
    );
    assert_eq!(auto["tool_choice"], "auto");
    let req = hs_loop::realmodel::build_body_messages(
        "kimi-k3",
        "sys",
        &messages,
        Some(&tools),
        None,
        true,
    );
    assert_eq!(req["tool_choice"], "required");
}
