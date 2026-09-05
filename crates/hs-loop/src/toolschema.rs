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
            "Run a command inside a sandbox with the full machine floor: network on, system roots writable, you are root; the live repo stays clean. A bare command runs as a general shell on a pristine copy (git log, grep, pwd - no diff required). Pass diff INLINE to test a candidate patch BEFORE writing any answer; pass path (or neither) to test the current answer file. If the patch does not apply you get the git error back free - fix the framing before spending a checker cycle. Run the FAIL_TO_PASS command before every answer.write.",
            json!({"type":"object","properties":{
                "command":{"type":"string"},
                "diff":{"type":"string","description":"unified diff, optional"},
                "path":{"type":"string","description":"ANSWER_PATH, optional"}},"required":["command"]}),
        ),
        f(
            "edit.apply",
            "Apply one incremental edit (unified diff) to your persistent candidate workspace - the live repo is never touched. Returns the CUMULATIVE diff of everything you have applied so far: use edit.apply as you work, test with repo.exec, and submit the cumulative result. Set op='diff' to re-read the cumulative diff, op='reset' to discard the candidate.",
            json!({"type":"object","properties":{
                "diff":{"type":"string","description":"unified diff"},
                "op":{"type":"string","enum":["diff","reset"],"description":"optional operation instead of applying a diff"}}}),
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
            "answer.write",
            "Submit your patch: content is one fenced unified diff (```diff ... ```, paths a/... b/... relative to repo root). Ground every hunk in code you actually read: correct file, correct current line numbers, exact context lines. Run the FAIL_TO_PASS command via repo.exec before every answer.write. The checker runs automatically after each answer.write and its verdict comes back as FEEDBACK.",
            json!({"type":"object","properties":{
                "path":{"type":"string","description":"the ANSWER_PATH value"},
                "content":{"type":"string","description":"```diff\n<one unified diff>\n```"}},"required":["path","content"]}),
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
