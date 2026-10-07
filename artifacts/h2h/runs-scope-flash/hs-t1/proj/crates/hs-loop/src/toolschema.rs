//! Native tool schemas (`OpenAI` function-calling shape) for the builtin
//! mission tool surface - the single source of truth for what the model is
//! told about its tools. Delivery is the provider API's `tools` parameter
//! (native tool calling), never hand-rendered prompt text (Eric 2026-09-05:
//! "no hand-rolled anything"; kimi-k3 native path verified live through the
//! production relay: `finish_reason=tool_calls`, `tool_choice:"required`" honored).

use serde_json::{json, Value};

fn f(name: &str, description: &str, parameters: Value) -> Value {
    json!({"type": "function", "function": {"name": name, "description": description, "parameters": parameters}})
}

/// The seven builtin mission tools. The behavioral guidance that used to
/// live in the hand-rendered TOOLS prompt block lives here, in the native
/// schema descriptions.
#[must_use]
pub fn builtin_tools() -> Vec<Value> {
    builtin_tools_with_edit("applypatch")
}

/// The builtin surface with the edit-path flavor selected (bake-off,
/// 2026-09-06): "applypatch" (edit.patch, Codex grammar) or "anchor"
/// (edit.anchor, hashline anchors + anchored repo.read).
#[must_use]
pub fn builtin_tools_with_edit(edit_path: &str) -> Vec<Value> {
    let mut tools = builtin_tools_inner();
    if edit_path == "anchor" {
        for t in &mut tools {
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
            "Run a command inside a sandbox with the full machine floor: network on, system roots writable, you are root; the live repo stays clean. EVERY call is a FRESH whole-filesystem sandbox: nothing persists between calls - not files you create (including /tmp), not packages you install, not shell state. Only the live repo and your edit.patch candidate persist across calls. Use python3 for Python (bare `python` may not exist). A bare command runs as a general shell on a pristine copy (git log, grep, pwd - no diff required). Pass diff INLINE to test a candidate patch BEFORE writing any answer; pass path (or neither) to test the current answer file. If the patch does not apply you get the git error back free - fix the framing before spending a checker cycle. Run the FAIL_TO_PASS command before every answer.submit. Make edits ONLY with edit.patch: git apply and writing .diff/.patch files here are rejected with a steering error; repeated attempts of the same class are counted and escalate.",
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


/// Terminal-bench tool surface (2026-09-07): the agent works DIRECTLY on the
/// live task container - term.exec replaces repo.exec + edit.patch (the
/// container is the sandbox; the graded artifact is machine state, not a
/// patch). checker.run is never model-visible: it runs automatically after
/// answer.submit. Same blind stop authority as swe blind mode: the agent's
/// own .hs/checks decide completion; official tests are hidden and external.
/// Eric's five #5: delegation tool, offered on the interactive
/// surface only (repl `native_tools`) - benchmark runners keep the
/// fixed tb tool set.
#[must_use]
pub fn agent_spawn_tool() -> Value {
    f(
        "agent.spawn",
        "Delegate a self-contained subtask to a child sub-agent that runs the same harness on its own stream CONCURRENTLY: the call returns as soon as the child is running (status running, its stream id), and the harness tells you when it finishes (a delegation update names passed, steps, cost). Delegate every separable subtask as soon as you know it, then keep working; never poll for completion yourself. Optionally name a different registered model for the child. The parent linkage and stream id are injected by the harness - never pass parent_stream or child_stream_id yourself.",
        json!({"type":"object","properties":{
            "mission":{"type":"string","description":"the delegated task, self-contained"},
            "model":{"type":"string","description":"optional: a registered model name for the child (default: the parent's default)"}},"required":["mission"]}),
    )
}

#[must_use]
pub fn agent_fork_tool() -> Value {
    f("agent.fork", "Like agent.spawn, but the child starts with your working context (files you read, edits, checks so far) so it continues from where you are instead of from scratch. Use for a branch of the same work.", json!({"type":"object","properties":{"mission":{"type":"string"},"model":{"type":"string"}},"required":["mission"]}))
}

#[must_use]
pub fn agent_list_tool() -> Value {
    f("agent.list", "List the sub-agents you delegated: child id, mission, state (running|done|lost), passed, steps.", json!({"type":"object","properties":{}}))
}

#[must_use]
pub fn agent_send_tool() -> Value {
    f("agent.send", "Send a message to a running sub-agent by child id; it sees it at its next step as operator steering.", json!({"type":"object","properties":{"child":{"type":"string"},"message":{"type":"string"}},"required":["child","message"]}))
}

#[must_use]
pub fn agent_interrupt_tool() -> Value {
    f("agent.interrupt", "Ask a running sub-agent to stop at its next step boundary (booked as interrupted, not a failure of the harness).", json!({"type":"object","properties":{"child":{"type":"string"}},"required":["child"]}))
}

#[must_use]
pub fn agent_spawn_poll_tool() -> Value {
    f(
        "agent.spawn_poll",
        "Harness-internal: query a delegated child's state (running, or done with its report). The loop polls this at step boundaries; models should never call it - a delegation update arrives on its own.",
        json!({"type":"object","properties":{
            "child_stream_id":{"type":"string","description":"the running child's stream id"}},"required":["child_stream_id"]}),
    )
}

#[must_use]
pub fn tb_tools() -> Vec<Value> {
    vec![
        f(
            "term.exec",
            "Run a bash command DIRECTLY in the live task container (you are root, network on). State PERSISTS between calls: files you write, packages you install, services you start all stay - this is the real machine the hidden tests inspect after you finish, so make your changes here, never in scratch copies. Returns exit_code + stdout/stderr tails. For services or long jobs, start them in the background (nohup ... &) and poll.",
            json!({"type":"object","properties":{
                "command":{"type":"string","description":"a bash command line, run from the task workdir"}},"required":["command"]}),
        ),
        f(
            "repo.search",
            "Find code by literal substring under the task workdir; returns path:line hits (max 100).",
            json!({"type":"object","properties":{"pattern":{"type":"string","description":"literal substring to find"}},"required":["pattern"]}),
        ),
        f(
            "repo.read",
            "Read a file window under the task workdir. The reply tells you total_lines and a truncated flag; if truncated, page forward with start_line=end_line+1. NEVER re-read the same window: recent results stay verbatim in your transcript. Put durable facts in notes.scratch.",
            json!({"type":"object","properties":{
                "path":{"type":"string","description":"workdir-relative path"},
                "start_line":{"type":"integer","description":"1-indexed first line, optional"},
                "max_lines":{"type":"integer","description":"optional, default 400"}},"required":["path"]}),
        ),
        f(
            "ask_user_question",
            "Ask the user one question and wait for the answer. Use only when a decision is genuinely theirs and you cannot proceed safely on a stated assumption. Optional options lists suggested answers. If no answer arrives you get an error: continue on your best assumption and say so.",
            json!({"type":"object","properties":{
                "question":{"type":"string"},
                "options":{"type":"array","items":{"type":"string"},"description":"optional suggested answers"}},"required":["question"]}),
        ),
        f(
            "jobs",
            "Background jobs in the same confined shell as term.exec. op=start {command} returns at once with an id; op=list shows status and exit codes; op=output {id, tail_bytes?} reads the log while it runs; op=kill {id} ends it. Use for servers, watchers and long builds instead of blocking a step.",
            json!({"type":"object","properties":{
                "op":{"type":"string","enum":["start","list","output","kill"]},
                "command":{"type":"string","description":"bash command line, for start"},
                "id":{"type":"string","description":"job id, for output/kill"},
                "tail_bytes":{"type":"integer","description":"output tail cap, default 20000"}},"required":["op"]}),
        ),
        f(
            "glob",
            "Find files by glob pattern (e.g. **/*.rs), relative to the workspace or an optional subpath. Honors .gitignore; sorted; limit default 200.",
            json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"},"limit":{"type":"integer"}},"required":["pattern"]}),
        ),
        f(
            "todo",
            "Your task list for multi-step work (todo_write). Pass todos=[{content, status: pending|in_progress|completed}] to replace the list (at most one in_progress); call with no todos to read it back.",
            json!({"type":"object","properties":{"todos":{"type":"array","items":{"type":"object","properties":{"content":{"type":"string"},"status":{"type":"string","enum":["pending","in_progress","completed"]}},"required":["content","status"]}}}}),
        ),
        f(
            "present",
            "Show a finished file from the workspace to the user (report, image, document). path is relative to the workspace; optional title.",
            json!({"type":"object","properties":{"path":{"type":"string"},"title":{"type":"string"}},"required":["path"]}),
        ),
        f(
            "read_image",
            "Read a PNG, JPEG or GIF from the workspace: returns mime, width, height and base64 data (max 5 MB).",
            json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
        ),
        f(
            "schedule",
            "Recurring prompts. op=create {prompt, every_secs>=60}, list, update {id, prompt?, every_secs?}, delete {id}, due (entries now due; advances them).",
            json!({"type":"object","properties":{"op":{"type":"string","enum":["create","list","update","delete","due"]},"id":{"type":"string"},"prompt":{"type":"string"},"every_secs":{"type":"integer"}},"required":["op"]}),
        ),
        f(
            "speak",
            "Voice: speak text aloud into a WAV file in the workspace with the configured text-to-speech voice. {text, path?} (relative .wav path, default .hs/voice/<time>.wav). Fails with a clear error when no voice is configured.",
            json!({"type":"object","properties":{"text":{"type":"string"},"path":{"type":"string"}},"required":["text"]}),
        ),
        f(
            "workflow",
            "Named multi-step workflows. op=define {name, steps: [prompt,...]}, get {name}, list, delete {name}. The harness runs a workflow's steps as goals back to back.",
            json!({"type":"object","properties":{"op":{"type":"string","enum":["define","get","list","delete"]},"name":{"type":"string"},"steps":{"type":"array","items":{"type":"string"}}},"required":["op"]}),
        ),
        f(
            "goal",
            "Track the long-running goal of this session. op=create {objective, acceptance?} (one active goal at a time), op=get, op=update {status: active|done|blocked, note?}. This is your own tracker; the mission still closes only through the checker.",
            json!({"type":"object","properties":{
                "op":{"type":"string","enum":["create","get","update"]},
                "objective":{"type":"string"},"acceptance":{"type":"string"},
                "status":{"type":"string","enum":["active","done","blocked"]},
                "note":{"type":"string"}},"required":["op"]}),
        ),
        f(
            "plan",
            "Plan mode. op=enter: mutating tools (term.exec, edits, jobs start, agent.spawn, answer.submit) are blocked while you research. op=exit {plan}: save your plan and leave plan mode so work can start. op=status reads the saved plan.",
            json!({"type":"object","properties":{
                "op":{"type":"string","enum":["enter","exit","status"]},
                "plan":{"type":"string","description":"the plan text, for exit"}},"required":["op"]}),
        ),
        f(
            "web.search",
            "Search the web (Brave Search API; needs HS_BRAVE_API_KEY). Returns title, url, snippet per result. Follow up with web.fetch to read a page.",
            json!({"type":"object","properties":{
                "query":{"type":"string"},
                "count":{"type":"integer","description":"results, 1-20, default 5"}},"required":["query"]}),
        ),
        f(
            "web.fetch",
            "Fetch an http(s) URL as text (HTML is reduced to readable text). Private, loopback and non-http targets are refused. Use for docs and API references; returns status, content_type, text, truncated.",
            json!({"type":"object","properties":{
                "url":{"type":"string","description":"http(s) URL"},
                "max_bytes":{"type":"integer","description":"optional cap, default 200000"}},"required":["url"]}),
        ),
        f(
            "notes.scratch",
            "Persistent notes that survive context truncation. Record hypotheses, commands that worked, and values you will need later; read them back instead of re-discovering.",
            json!({"type":"object","properties":{
                "op":{"type":"string","enum":["write","append","read"]},
                "content":{"type":"string","description":"text for write/append"}},"required":["op"]}),
        ),
        f(
            "answer.submit",
            "Finish the task: writes your completion summary (summary: what you changed and how you verified it) to ANSWER_PATH and triggers the checker, which runs YOUR .hs/checks against the live machine. Green ends the mission; red comes back as FEEDBACK. Your checks must be declared when you submit: write .hs/checks yourself in an earlier step, or pass them atomically in the optional checks field (one command per line, persisted to .hs/checks BEFORE the checker runs). Submit only when every check you declared passes.",
            json!({"type":"object","properties":{
                "path":{"type":"string","description":"the ANSWER_PATH value"},
                "summary":{"type":"string","description":"what you changed and how you verified it"},
                "checks":{"type":"string","description":"optional: your verification commands, one per line; persisted to .hs/checks BEFORE the checker runs on this submission"}},"required":["path","summary"]}),
        ),
    ]
}


/// Generic fallback schema for a registry tool with no authored schema:
/// the menu must still name it so advertised == dispatchable (D1).
fn generic_schema(name: &str) -> Value {
    f(
        name,
        "Registered plugin tool; call with a JSON args object (see the harness contract for this tool's argument shape).",
        json!({"type":"object"}),
    )
}

/// Authored schema for one registry tool name, or None when the name has
/// no hand-written schema (`generic_schema` covers it). Keyed by the REAL
/// plugin name so the advertised surface derives from the registry (D1,
/// dance #94) - never from a flavor-seeded list that can drift apart.
#[must_use]
pub fn schema_for(name: &str, edit_path: &str) -> Option<Value> {
    // Reuse the authored builders: both flavor lists contain per-name
    // schemas; builtin carries repo.exec/edit.*, tb carries term.exec.
    let authored = builtin_tools_with_edit(edit_path)
        .into_iter()
        .chain(tb_tools())
        .collect::<Vec<_>>();
    if name == "answer.submit" {
        // Mode-correct schema (live finding, GLM-5.3): the plugin requires
        // {path, summary} in plain REPL mode (the summary IS the answer
        // content) and {path} only under HS_SWE_WORKSPACE / HS_ANSWER_RAW,
        // where the harness computes the deliverable itself. The builtin
        // list holds the SWE variant and would always win the find below,
        // so pick by mode here.
        // The discriminator is HS_ANSWER_RAW, not HS_SWE_WORKSPACE: the
        // REPL sets HS_SWE_WORKSPACE for EVERY session (plugin env
        // anchoring); live-machine configs (term.exec) add HS_ANSWER_RAW=1,
        // and THAT is what switches the plugin to summary-required.
        let want_swe = std::env::var("HS_ANSWER_RAW").as_deref() != Ok("1");
        return authored.into_iter().find(|t| {
            if t["function"]["name"].as_str() != Some(name) {
                return false;
            }
            let has_summary = t["function"]["parameters"]["properties"]
                .as_object()
                .is_some_and(|p| p.contains_key("summary"));
            has_summary != want_swe
        });
    }
    authored
        .into_iter()
        .find(|t| t["function"]["name"].as_str() == Some(name))
}

/// D1: derive the model-facing surface from the kernel's registry.
/// `registered` = `kernel.list_tools(subject)` names; `discovered` = MCP
/// native schemas (server-authored, keyed by `mcp.<server>.<tool>`); every
/// other registered name uses its authored schema or the generic
/// fallback. The result contains EXACTLY the registered names - no more,
/// no fewer - so advertised == dispatchable by construction.
#[must_use]
pub fn schemas_for_registry(
    registered: &[String],
    discovered: &[Value],
    edit_path: &str,
) -> Vec<Value> {
    let mut out = Vec::with_capacity(registered.len());
    for name in registered {
        if let Some(t) = discovered
            .iter()
            .find(|t| t["function"]["name"].as_str() == Some(name.as_str()))
        {
            out.push(t.clone());
        } else if let Some(t) = schema_for(name, edit_path) {
            out.push(t);
        } else {
            out.push(generic_schema(name));
        }
    }
    out
}

/// One MCP-discovered tool in native shape. The input schema comes from the
/// server's tools/list verbatim; an absent schema degrades to an open object.
#[must_use]
pub fn mcp_tool(name: &str, description: &str, input_schema: Option<Value>) -> Value {
    let params = input_schema.unwrap_or_else(|| json!({"type":"object","properties":{}}));
    f(name, description, params)
}

/// Provider constraint (verified live 2026-09-05, Moonshot `invalid_request_error`:
/// "function name is invalid, must start with a letter and can contain
/// letters, numbers, underscores, and dashes"; `OpenAI`'s own schema is the
/// same charset): dotted tool names are not wire-legal. Map "." -> "__" on
/// the way out and back on the way in. Collision-free for this surface:
/// no builtin or MCP tool segment contains a double underscore.
#[must_use]
pub fn wire_name(name: &str) -> String {
    name.replace('.', "__")
}

#[must_use]
pub fn internal_name(wire: &str) -> String {
    wire.replace("__", ".")
}

/// The tools array with every function name rewritten to wire-legal form.
#[must_use]
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
#[must_use]
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

/// B1 (v5 D3 + cut #10): the K memory plane as a consulted tool. The
/// agent decides when to recall; nothing from earlier missions is
/// pre-passed into its context. Retrieval runs against the session's
/// attached `MemoryStore` inside the loop (never the plugin bus); every
/// record carries `source_seqs` provenance into the event log.
#[must_use]
pub fn memory_recall_tool() -> Value {
    f(
        "memory.recall",
        "Query K, the cross-mission typed memory plane (design D3). Returns top-k records from EARLIER missions by importance then recency - what the team has already learned - each with source_seqs pointing into the event log so you can chase provenance. Nothing from prior missions reaches you automatically: call this BEFORE re-deriving anything a past mission may have established. When a served record actually informs your work, CITE its id verbatim in your visible output (notes, edits, submission): cited records gain reward, served-but-uncited records lose it - the ledger is how K learns what is worth keeping. k defaults to 5.",
        json!({"type":"object","properties":{"k":{"type":"integer","description":"records to return, 1..=50 (default 5)"}}}),
    )
}

/// B2 (v5 gate 6, spec 3.4): the shared world as mission tools. The agent
/// WRITES proposals; the world service alone validates them (schema,
/// permissions, material preconditions, quarantine of the proposing
/// stream) and writes the consequences. Zero-message coordination: other
/// agents reuse installed artifacts by observation, never by messages.
#[must_use]
pub fn world_propose_tool() -> Value {
    f(
        "world.propose",
        "Propose an artifact into the shared world (gate 6): your proposal is validated by the world service (schema + content hash + path + version), which alone writes the consequence. Returns the validated artifact_id/version on success, or the rejection reason. Content is hashed and stored content-addressed; world_path MUST be absolute; kind: file|program|controller|note|skill. Installed controllers keep acting on world ticks even after your stream ends (executable inheritance).",
        json!({"type":"object","properties":{
            "world_path":{"type":"string","description":"absolute path in the shared world"},
            "content":{"type":"string","description":"artifact content (controllers: the declarative program JSON, e.g. {\"op\":\"append_counter\",\"target\":\"/path\"})"},
            "kind":{"type":"string","enum":["file","program","controller","note","skill"]},
            "artifact_id":{"type":"string","description":"uuid, optional (generated when omitted)"},
            "parent_version":{"type":"string","description":"uuid of the artifact version this proposal mutates, optional (recorded provenance: mutation parents)"},
            "version":{"type":"integer","description":"optional, default 1"}},"required":["world_path","content"]}),
    )
}

/// B2: zero-message coordination - read the current live (validated or
/// installed) artifacts at a world path. This is how a later mission
/// reuses an earlier mission's work with no messages at all.
#[must_use]
pub fn world_observe_tool() -> Value {
    f(
        "world.observe",
        "Read the shared world (gate 6): the live (validated or installed) artifacts at a world path - what other missions have ALREADY delivered there. Zero-message coordination: consult the world BEFORE re-deriving anything another mission may have produced. Every observe that delivers artifacts is booked as reuse by your stream, and each result row reports reuse_count - the culture's diffusion count.",
        json!({"type":"object","properties":{"world_path":{"type":"string"}},"required":["world_path"]}),
    )
}

/// B2: install a validated controller/program artifact. It starts acting
/// on every world tick WITHOUT any model call, and it keeps acting after
/// your stream ends - an installed controller is world property
/// (executable inheritance).
#[must_use]
pub fn world_install_tool() -> Value {
    f(
        "world.install",
        "Install a validated controller or program artifact by artifact_id (gate 6). Installed artifacts act on every world tick with no model call and keep acting after your stream ends. Only validated controller/program artifacts install.",
        json!({"type":"object","properties":{"artifact_id":{"type":"string"}},"required":["artifact_id"]}),
    )
}

/// B2: advance the world one tick - every installed controller acts once,
/// purely mechanically (no model call). Returns the consequence event
/// artifact ids this tick produced.
#[must_use]
pub fn world_tick_tool() -> Value {
    f(
        "world.tick",
        "Advance the shared world one tick (gate 6): every installed controller acts once, mechanically, with no model call. Returns the artifact ids this tick produced.",
        json!({"type":"object","properties":{}}),
    )
}

/// Hermes-style skills: list installed world skills (name + description).
#[must_use]
pub fn skill_list_tool() -> Value {
    f(
        "skill.list",
        "List the installed world skills (name and one-line description). Skills are reusable, verified procedures; the prompt shows the same index.",
        json!({"type":"object","properties":{}}),
    )
}

/// Load an installed skill's full SKILL.md instructions by exact name.
#[must_use]
pub fn skill_view_tool() -> Value {
    f(
        "skill.view",
        "Load the full instructions of an installed skill by exact name (from the Installed skills index). Call it when a skill's description matches your task, before re-deriving the procedure.",
        json!({"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}),
    )
}
