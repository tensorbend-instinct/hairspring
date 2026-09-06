//! Native tool schemas (OpenAI function-calling shape) for the builtin
//! mission tool surface - the single source of truth for what the model is
//! told about its tools. Delivery is the provider API's `tools` parameter
//! (native tool calling), never hand-rendered prompt text (Eric 2026-09-05:
//! "no hand-rolled anything"; kimi-k3 native path verified live through the
//! production relay: finish_reason=tool_calls, tool_choice:"required" honored).

use serde_json::{json, Value};

fn f(name: &str, description: &str, parameters: Value) -> Value {
    json!({"type": "function", "function": {"name": name, "description": description, "parameters": parameters}})
}

/// The seven builtin mission tools. The behavioral guidance that used to
/// live in the hand-rendered TOOLS prompt block lives here, in the native
/// schema descriptions.
pub fn builtin_tools() -> Vec<Value> {
    builtin_tools_with_edit("applypatch")
}

/// The builtin surface with the edit-path flavor selected (bake-off,
/// 2026-09-06): "applypatch" (edit.patch, Codex grammar) or "anchor"
/// (edit.anchor, hashline anchors + anchored repo.read).
pub fn builtin_tools_with_edit(edit_path: &str) -> Vec<Value> {
    let mut tools = builtin_tools_inner();
    if edit_path == "anchor" {
        for t in tools.iter_mut() {
            match t["function"]["name"].as_str() {
                Some("edit.patch") => {
                    *t = f(
                        "edit.anchor",
                        "Edit your persistent candidate workspace by anchors - the live repo is never touched. repo.read shows every line as LINE:HASH\u{2192}content; quote those anchors back exactly. edits: [{op:\"replace\", anchor, end_anchor?, content} (empty content deletes), {op:\"insert_after\", anchor (\"0:\"=top of file, \"EOF\"=end), content}, {op:\"write\", content} (whole file; the only way to create a new file)]. Anchors are validated against the CURRENT candidate at apply time: a stale or wrong anchor is a named error listing the failed anchors - re-read the file and re-quote; nothing is half-applied. Returns a fresh-anchored snippet of the edited region plus the CUMULATIVE diff of everything applied so far: test with repo.exec, submit with answer.submit. Set op='diff' to re-read the cumulative diff, op='reset' to discard the candidate.",
                        json!({"type":"object","properties":{
                            "path":{"type":"string","description":"repo-relative file path"},
                            "edits":{"type":"array","items":{"type":"object"},"description":"anchor-typed ops, applied bottom-up after full validation"},
                            "op":{"type":"string","enum":["diff","reset"],"description":"optional operation instead of applying edits"}}}),
                    );
                }
                Some("repo.read") => {
                    *t = f(
                        "repo.read",
                        "Read a file window of YOUR CANDIDATE (your edits are visible here). Every line is shown as LINE:HASH\u{2192}content - quote the LINE:HASH anchor back to edit.anchor. The reply tells you total_lines and a truncated flag; if truncated, page forward with start_line=end_line+1. NEVER re-read the same window: recent results stay verbatim in your transcript. Put durable facts (hypotheses, line numbers, failing tests) in notes.scratch.",
                        json!({"type":"object","properties":{
                            "path":{"type":"string","description":"repo-relative path"},
                            "start_line":{"type":"integer","description":"1-indexed first line, optional"},
                            "max_lines":{"type":"integer","description":"optional, default 400"}},"required":["path"]}),
                    );
                }
                _ => {}
            }
        }
    }
    tools
}

fn builtin_tools_inner() -> Vec<Value> {
    vec![
        f(
            "repo.search",
            "Find code by literal substring; returns path:line hits (max 100). Honors the workspace .gitignore/.ignore files.",
            json!({"type":"object","properties":{"pattern":{"type":"string","description":"literal substring to find"}},"required":["pattern"]}),
        ),
        f(
            "repo.read",
            "Read a file window. The reply tells you total_lines and a truncated flag; if truncated, page forward with start_line=end_line+1. NEVER re-read the same window: recent results stay verbatim in your transcript. Put durable facts (hypotheses, line numbers, failing tests) in notes.scratch.",
            json!({"type":"object","properties":{
                "path":{"type":"string","description":"repo-relative path"},
                "start_line":{"type":"integer","description":"1-indexed first line, optional"},
                "max_lines":{"type":"integer","description":"optional, default 400"}},"required":["path"]}),
        ),
        f(
            "repo.exec",
            "Run a command inside a sandbox with the full machine floor: network on, system roots writable, you are root; the live repo stays clean. A bare command runs as a general shell on a pristine copy (git log, grep, pwd - no diff required). Pass diff INLINE to test a candidate patch BEFORE writing any answer; pass path (or neither) to test the current answer file. If the patch does not apply you get the git error back free - fix the framing before spending a checker cycle. Run the FAIL_TO_PASS command before every answer.submit. Make edits ONLY with edit.patch: git apply and writing .diff/.patch files here are rejected with a steering error; repeated attempts of the same class are counted and escalate.",
            json!({"type":"object","properties":{
                "command":{"type":"string"},
                "diff":{"type":"string","description":"unified diff, optional"},
                "path":{"type":"string","description":"ANSWER_PATH, optional"}},"required":["command"]}),
        ),
        f(
            "edit.patch",
            "Edit your persistent candidate workspace with the Codex apply_patch grammar - the live repo is never touched. patch text: *** Begin Patch, then per file one of: *** Update File: path (an @@ context line, then -old/+new lines copied verbatim from repo.read), *** Add File: path (+lines), *** Delete File: path; close with *** End Patch. NO line numbers, NO unified-diff syntax. Context must match the CURRENT candidate exactly: a mismatch is a named error and the candidate stays untouched. Returns the CUMULATIVE diff of everything applied so far: test with repo.exec, submit with answer.submit. Set op='diff' to re-read the cumulative diff, op='reset' to discard the candidate.",
            json!({"type":"object","properties":{
                "patch":{"type":"string","description":"one apply_patch text: *** Begin Patch ... *** End Patch"},
                "op":{"type":"string","enum":["diff","reset"],"description":"optional operation instead of applying a patch"}}}),
        ),
        f(
            "notes.scratch",
            "Persistent notes that survive context truncation. Record hypotheses, failing test names, and line numbers you will need later; read them back instead of re-discovering.",
            json!({"type":"object","properties":{
                "op":{"type":"string","enum":["write","append","read"]},
                "content":{"type":"string","description":"text for write/append"}},"required":["op"]}),
        ),
        f(
            "policy.propose_prompt",
            "Propose a better operating prompt for FUTURE missions. Recorded, versioned, and reviewed through the gated promotion path; it never changes this mission.",
            json!({"type":"object","properties":{
                "name":{"type":"string","description":"prompt name, e.g. swe-mission"},
                "text":{"type":"string","description":"your improved prompt template"}},"required":["name","text"]}),
        ),
        f(
            "answer.submit",
            "Submit your fix: computes the unified diff of your candidate workspace with git and writes it to ANSWER_PATH. No content argument - the submission is exactly what you built with edit.patch and verified with repo.exec; hand-written diff text is never accepted. Fails with a steering error when the candidate has no edits. Run the FAIL_TO_PASS command via repo.exec before every answer.submit. The checker runs automatically after each answer.submit and its verdict comes back as FEEDBACK.",
            json!({"type":"object","properties":{
                "path":{"type":"string","description":"the ANSWER_PATH value"}},"required":["path"]}),
        ),
    ]
}

/// One MCP-discovered tool in native shape. The input schema comes from the
/// server's tools/list verbatim; an absent schema degrades to an open object.
pub fn mcp_tool(name: &str, description: &str, input_schema: Option<Value>) -> Value {
    let params = input_schema.unwrap_or_else(|| json!({"type":"object","properties":{}}));
    f(name, description, params)
}

/// Provider constraint (verified live 2026-09-05, Moonshot invalid_request_error:
/// "function name is invalid, must start with a letter and can contain
/// letters, numbers, underscores, and dashes"; OpenAI's own schema is the
/// same charset): dotted tool names are not wire-legal. Map "." -> "__" on
/// the way out and back on the way in. Collision-free for this surface:
/// no builtin or MCP tool segment contains a double underscore.
pub fn wire_name(name: &str) -> String {
    name.replace('.', "__")
}

pub fn internal_name(wire: &str) -> String {
    wire.replace("__", ".")
}

/// The tools array with every function name rewritten to wire-legal form.
pub fn to_wire(tools: &Value) -> Value {
    let mut out = tools.clone();
    if let Some(arr) = out.as_array_mut() {
        for t in arr.iter_mut() {
            if let Some(n) = t["function"]["name"].as_str() {
                t["function"]["name"] = json!(wire_name(n));
            }
        }
    }
    out
}

/// The verifier's verdict as a native tool (user directive 2026-09-05: the
/// verdict path migrates from text-JSON to a native tool call - the
/// response-format contract lives in this schema, not in prompt prose).
pub fn verdict_tool() -> Value {
    f(
        "verdict.submit",
        "Submit the audit verdict exactly once. Default to refuted when uncertain a required criterion holds; never invent requirements. Audit the RECORDED evidence only - a prose claim of test output with no recorded run is fabricated: refute.",
        json!({"type":"object","properties":{
            "refuted":{"type":"boolean","description":"true when the recorded evidence fails the audit"},
            "blocking":{"type":"string","enum":["none","contradiction","unverifiable"]},
            "findings":{"type":"array","items":{"type":"object","properties":{
                "kind":{"type":"string","enum":["bug","gap","todo"]},
                "location":{"type":"string"},
                "detail":{"type":"string","description":"one line"}},
                "required":["kind","location","detail"]}}
        },"required":["refuted","blocking","findings"]}),
    )
}
