//! RED-first: native tool-calling migration (Eric 2026-09-05, non-negotiable:
//! no hand-rolled tool layer). Verified live against kimi-k3 through the
//! production relay: native tools + tool_choice:"required" return
//! finish_reason="tool_calls" with name+arguments (probe, 2026-09-05).
//! The prompt must carry NO hand-rolled tool markup; tool delivery is the
//! provider API's tools parameter; responses parse through the native
//! tool_calls path; repo.search honors ignore files.

use serde_json::json;

fn dummy_args() -> hs_loop::sweprompt::PromptArgs {
    hs_loop::sweprompt::PromptArgs {
        ws: "/tmp/ws".into(),
        problem_statement: "p".into(),
        fail_to_pass: vec!["t".into()],
        repo_layout: "f".into(),
        nudge: String::new(),
        answer_path: "/tmp/a".into(),
        orientation: "python3".into(),
        mcp_tools: "mcp.graft.graft_repo_map - MCP tool\n".into(),
    }
}

#[test]
fn prompt_has_no_handrolled_tool_markup() {
    let p = hs_loop::sweprompt::build_mission_prompt(None, &dummy_args());
    assert!(!p.contains("{{"), "doubled-brace format leak in prompt");
    assert!(
        !p.contains("\"tool\":"),
        "hand-rolled tool-call template in prompt"
    );
    assert!(
        !p.contains("TOOLS (one tool call per reply"),
        "hand-rendered TOOLS section in prompt"
    );
    assert!(
        !p.contains("mcp.graft"),
        "MCP tools are delivered via the native tools parameter, not prompt text"
    );
}

#[test]
fn builtin_tool_schemas_cover_the_mission_surface() {
    let tools = hs_loop::toolschema::builtin_tools();
    let names: Vec<&str> = tools
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap_or(""))
        .collect();
    for want in [
        "repo.search",
        "repo.read",
        "repo.exec",
        "edit.apply",
        "notes.scratch",
        "policy.propose_prompt",
        "answer.write",
    ] {
        assert!(names.contains(&want), "missing native schema for {want}");
    }
    for t in &tools {
        assert_eq!(t["type"].as_str(), Some("function"), "OpenAI shape: {t}");
        assert_eq!(
            t["function"]["parameters"]["type"].as_str(),
            Some("object"),
            "schema parameters object: {t}"
        );
        assert!(
            t["function"]["description"].as_str().unwrap_or("").len() > 20,
            "every tool carries its behavioral description: {t}"
        );
    }
    let search = tools
        .iter()
        .find(|t| t["function"]["name"] == "repo.search")
        .unwrap();
    assert_eq!(search["function"]["parameters"]["required"], json!(["pattern"]));
    let write = tools
        .iter()
        .find(|t| t["function"]["name"] == "answer.write")
        .unwrap();
    assert_eq!(
        write["function"]["parameters"]["required"],
        json!(["path", "content"])
    );
}

#[test]
fn request_body_carries_native_tools() {
    let tools = json!([{"type":"function","function":{"name":"repo.search","description":"d","parameters":{"type":"object","properties":{"pattern":{"type":"string"}},"required":["pattern"]}}}]);
    let body = hs_loop::realmodel::build_body("kimi-k3", "sys", "prompt", Some(&tools), None);
    // provider name-charset constraint (verified live: Moonshot rejects dots):
    // names go out in wire form, schemas otherwise verbatim
    assert_eq!(
        body["tools"][0]["function"]["name"].as_str(),
        Some("repo__search"),
        "wire name mapped: {}",
        body["tools"][0]["function"]["name"]
    );
    assert_eq!(body["tools"][0]["function"]["parameters"], tools[0]["function"]["parameters"]);
    assert_eq!(
        body["tool_choice"].as_str(),
        Some("required"),
        "one tool call per reply, enforced by the API"
    );
    let bare = hs_loop::realmodel::build_body("kimi-k3", "sys", "prompt", None, None);
    assert!(bare.get("tools").is_none(), "no tools param without tools");
    assert!(bare.get("tool_choice").is_none());
}

#[test]
fn native_tool_call_roundtrip_parses() {
    let resp = json!({
        "choices": [{"finish_reason": "tool_calls", "message": {"content": "",
            "tool_calls": [{"index": 0, "id": "x", "type": "function",
                "function": {"name": "repo__read", "arguments": "{\"path\":\"a.py\"}"}}]}}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 20,
                  "completion_tokens_details": {"reasoning_tokens": 15}}
    });
    let p = hs_loop::realmodel::glm();
    let out = hs_loop::realmodel::parse_response(&p, &resp).expect("native tool call parses");
    let completion: serde_json::Value = serde_json::from_str(&out.completion).unwrap();
    assert_eq!(completion, json!({"tool": "repo.read", "args": {"path": "a.py"}}));
    assert_eq!(out.input_tokens, 10);
    assert_eq!(out.output_tokens, 20);
    assert_eq!(out.reasoning_tokens, 15, "reasoning usage surfaced for the log");
}

#[test]
fn no_tool_calls_is_an_error_not_a_text_fallback() {
    // native-only (Eric: no hand-rolled fallback layer): a content-only
    // reply - even one containing a JSON tool call as text - is an error
    // that feeds the retry path, never a silent second protocol.
    let resp = json!({
        "choices": [{"finish_reason": "stop", "message": {"content": "{\"tool\":\"repo.search\",\"args\":{\"pattern\":\"x\"}}"}}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 20}
    });
    let p = hs_loop::realmodel::glm();
    assert!(
        hs_loop::realmodel::parse_response(&p, &resp).is_err(),
        "content-only reply must not be honored on the native path"
    );
}

#[test]
fn search_repo_honors_ignore_files() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::create_dir_all(ws.join("a")).unwrap();
    std::fs::create_dir_all(ws.join("graft/.cache")).unwrap();
    std::fs::create_dir_all(ws.join("graft/cards")).unwrap();
    std::fs::create_dir_all(ws.join("ign")).unwrap();
    std::fs::write(ws.join("a/hit.py"), "PAT\n").unwrap();
    std::fs::write(ws.join("graft/.cache/f.json"), "PAT\n").unwrap();
    std::fs::write(ws.join("graft/cards/x.md"), "PAT\n").unwrap();
    std::fs::write(ws.join("ign/d.py"), "PAT\n").unwrap();
    std::fs::write(ws.join(".ignore"), "!graft/\ngraft/.cache/\n").unwrap();
    std::fs::write(ws.join(".gitignore"), "ign/\n").unwrap();
    let out = hs_loop::repotools::search_repo(ws, "PAT").unwrap();
    let mut paths: Vec<String> = out["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["path"].as_str().unwrap().to_string())
        .collect();
    paths.sort();
    assert_eq!(
        paths,
        vec!["a/hit.py".to_string(), "graft/cards/x.md".to_string()],
        ".gitignore and .ignore (incl. whitelist + re-ignore) must bind repo.search"
    );
}

#[test]
fn wire_names_are_provider_legal_and_round_trip() {
    for t in hs_loop::toolschema::builtin_tools() {
        let n = t["function"]["name"].as_str().unwrap();
        let w = hs_loop::toolschema::wire_name(n);
        assert!(
            w.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "wire-legal charset: {w}"
        );
        assert_eq!(hs_loop::toolschema::internal_name(&w), n, "round trip: {n}");
    }
    let dotted = "mcp.graft.graft_repo_map";
    assert_eq!(hs_loop::toolschema::wire_name(dotted), "mcp__graft__graft_repo_map");
    assert_eq!(
        hs_loop::toolschema::internal_name("mcp__graft__graft_repo_map"),
        dotted
    );
}
