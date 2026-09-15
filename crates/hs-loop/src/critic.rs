//! The CRITIC self-check standard (Eric 2026-09-07: author/critic
//! separation). The author writes and greens its own .hs/checks; then an
//! INDEPENDENT critic - a fresh model context that never saw the authoring
//! session - gets the original instruction, the declared checks, and shell
//! access, with one directive: REFUTE the submission. Instruction-anchored
//! hard assertions are extracted from the task text; every computed value
//! is re-derived by a DIFFERENT method than the declared checks use. A
//! submission passes only when the author's checks are green AND the critic
//! cannot refute it.
//!
//! Fail-closed everywhere: step cap, wall cap, budget cap, transport
//! error, malformed verdict - each means NOT passed. A capped or broken
//! critic is not a clean critic.
//!
//! The critic's shell is READ-ONLY BY MECHANISM, not by prompt courtesy:
//! `term_exec` routes through `termexec::run_readonly` (uid nobody, groups
//! cleared), so it can probe and re-derive but cannot modify the
//! submission under review (deep pass 2026-09-09: mechanism was an
//! unrestricted root shell behind a read-only prompt - closed).

use serde_json::{json, Value};
use std::path::Path;
use std::time::Instant;

/// One tool the critic may call: a shell command on the live machine.
pub const TERM_EXEC_TOOL: &str = "term_exec";

pub const CRITIC_SYSTEM: &str = "You are an independent verifier reviewing a finished submission on a live Linux machine. You did NOT do the work under review and you have no memory of how it was produced. Your only job is to try to REFUTE the claim that the submission satisfies the task. You have one tool, term_exec: it runs a shell command on the live machine as an UNPRIVILEGED user (cwd is the task workdir; scratch in /tmp persists between calls).\n\
METHOD, in order:\n\
1. Extract every hard requirement from the task instruction: required files, paths, formats, labels, units, counts, and numeric ranges. Test EACH ONE against the live machine state. Do not trust the declared checks' coverage - test the instruction, not the checks.\n\
2. For every computed value in the submission, re-derive it by a DIFFERENT method than the declared checks use: a different formula, an independent code path, or a back-calculation from the outputs. Both methods must agree.\n\
3. Probe the edges the declared checks ignore: missing files, units, rounding, ordering, extra or missing lines.\n\
RULES:\n\
- The task's files are read-only to you BY MECHANISM: you run unprivileged (not root), so any modify, move, or delete of them fails with permission denied. Scratch work goes in /tmp.\n\
- A refutation must be concrete and reproduced: name the command you ran and the output that proves the failure.\n\
- When you are done, reply with exactly one JSON object and nothing else: {\"refuted\": true, \"reason\": \"<reproduced failure, quoting command and output>\"} or {\"refuted\": false, \"reason\": \"<what you tested and re-derived>\"}. Reply refuted:false only after genuinely running steps 1-3.";

/// One model reply: either tool calls to run, or the final verdict text.
pub enum CriticReply {
    /// (call id, shell command) pairs.
    ToolCalls(Vec<(String, String)>),
    Final(String),
}

impl std::fmt::Debug for CriticReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CriticReply::ToolCalls(cs) => write!(f, "ToolCalls({cs:?})"),
            CriticReply::Final(t) => write!(f, "Final({t:?})"),
        }
    }
}

/// The critic's model interface. Scripted in tests, `DeepSeek` in prod.
pub trait CriticModel {
    fn step(&mut self, messages: &[Value]) -> Result<CriticReply, String>;
    /// Cumulative (`input_tokens`, `output_tokens`, `cost_micros`).
    fn usage(&self) -> (u64, u64, u64) {
        (0, 0, 0)
    }
}

#[derive(Clone, Debug)]
pub struct RefuteConfig {
    pub max_steps: u32,
    pub wall_secs: u64,
    pub budget_micros: u64,
    pub cmd_timeout_secs: u64,
}

impl Default for RefuteConfig {
    fn default() -> Self {
        Self { max_steps: 48, wall_secs: 1800, budget_micros: 1_000_000, cmd_timeout_secs: 120 }
    }
}

impl RefuteConfig {
    #[must_use]
    pub fn from_env() -> Self {
        let d = Self::default();
        let g = |k: &str, cur: u64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(cur);
        Self {
            max_steps: g("HS_CRITIC_MAX_STEPS", u64::from(d.max_steps)) as u32,
            wall_secs: g("HS_CRITIC_WALL_SECS", d.wall_secs),
            budget_micros: g("HS_CRITIC_BUDGET_MICROS", d.budget_micros),
            cmd_timeout_secs: g("HS_CRITIC_CMD_TIMEOUT_SECS", d.cmd_timeout_secs),
        }
    }
}

#[derive(Debug)]
pub struct RefuteOutcome {
    pub passed: bool,
    pub reason: String,
    pub steps: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_micros: u64,
    pub trace: Vec<Value>,
}

/// Lenient verdict parse: first JSON object in the text carrying a bool
/// "refuted". The model is told to reply with only the object; prose
/// around it is tolerated, a missing/malformed object is not.
#[must_use]
pub fn parse_verdict(text: &str) -> Option<(bool, String)> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    let v: Value = serde_json::from_str(&text[start..=end]).ok()?;
    let refuted = v["refuted"].as_bool()?;
    let reason = v["reason"].as_str().unwrap_or("").to_string();
    Some((refuted, reason))
}

fn tail(s: &str, n: usize) -> String {
    crate::msgfmt::tail_bytes_safe(s, n)
}

/// Run the refutation loop. Never panics into a pass: every abnormal exit
/// is passed:false with the reason.
pub fn refute(
    workdir: &Path,
    instruction: &str,
    checks_text: &str,
    cfg: &RefuteConfig,
    model: &mut dyn CriticModel,
) -> RefuteOutcome {
    let mut trace: Vec<Value> = vec![];
    let started = Instant::now();
    let mut messages = vec![
        json!({"role": "system", "content": CRITIC_SYSTEM}),
        json!({"role": "user", "content": format!(
            "TASK INSTRUCTION (verbatim):\n{instruction}\n\nTHE SUBMISSION UNDER REVIEW declares these checks (already green):\n{checks_text}\n\nWorkdir: {wd}. Refute the submission or clear it.",
            wd = workdir.display()
        )}),
    ];
    let mut steps: u32 = 0;
    let mut probes: u32 = 0;
    // Burn-down (critic precision, DISC matrix 2026-09-09: 13 fail-closed
    // false vetoes vs 1 real catch, "unparseable" the top cause): ONE
    // malformed verdict earns a schema-tightening retry instead of an
    // instant fail-closed. A second consecutive malformed verdict is a
    // genuinely broken critic and still fails closed.
    let mut verdict_retried = false;
    macro_rules! out {
        ($passed:expr_2021, $reason:expr_2021) => {{
            let (i, o, c) = model.usage();
            return RefuteOutcome {
                passed: $passed,
                reason: $reason,
                steps,
                input_tokens: i,
                output_tokens: o,
                cost_micros: c,
                trace,
            };
        }};
    }
    loop {
        if steps >= cfg.max_steps {
            out!(false, format!("critic hit its step cap ({}) without a verdict - fail-closed", cfg.max_steps));
        }
        if started.elapsed().as_secs() >= cfg.wall_secs {
            out!(false, format!("critic hit its wall cap ({}s) without a verdict - fail-closed", cfg.wall_secs));
        }
        let (_, _, cost) = model.usage();
        if cost >= cfg.budget_micros {
            out!(false, format!("critic hit its budget cap (${:.2}) without a verdict - fail-closed", cfg.budget_micros as f64 / 1e6));
        }
        // Reserve the final model call for a verdict. Without this boundary an
        // investigative critic can spend every step on probes and never answer.
        // Keep two calls at the end for convergence: the first requests a
        // verdict; if the model still emits tools, reject them without
        // execution and use the second call for a schema-tight retry.
        let final_only = steps.saturating_add(2) >= cfg.max_steps;
        if final_only {
            messages.push(json!({"role": "user", "content":
                "FINAL STEP. Do not call tools. Reply with EXACTLY one JSON object and nothing else: {\"refuted\": true, \"reason\": \"<reproduced failure>\"} or {\"refuted\": false, \"reason\": \"<what you tested and re-derived>\"}."
            }));
        }
        steps += 1;
        let reply = match model.step(&messages) {
            Ok(r) => r,
            Err(e) => out!(false, format!("critic model error: {e} - fail-closed")),
        };
        match reply {
            CriticReply::Final(text) => {
                trace.push(json!({"kind": "verdict", "text": text}));
                match parse_verdict(&text) {
                    Some((true, reason)) => out!(false, format!("critic refuted the submission: {reason}")),
                    Some((false, _)) if probes == 0 => out!(
                        false,
                        "critic returned a clean verdict without a single machine probe - fail-closed".to_string()
                    ),
                    Some((false, reason)) => out!(true, reason),
                    None => {
                        if verdict_retried {
                            out!(false, "critic verdict unparseable after retry - fail-closed".to_string());
                        }
                        verdict_retried = true;
                        trace.push(json!({"kind": "verdict_retry", "text": text}));
                        messages.push(json!({"role": "assistant", "content": text}));
                        messages.push(json!({"role": "user", "content":
                            "Your verdict was not parseable. Reply with EXACTLY one JSON object and nothing else: {\"refuted\": true, \"reason\": \"<reproduced failure, quoting command and output>\"} or {\"refuted\": false, \"reason\": \"<what you tested and re-derived>\"}. No prose, no markdown fences."
                        }));
                        continue;
                    }
                }
            }
            CriticReply::ToolCalls(calls) => {
                if final_only {
                    if steps >= cfg.max_steps {
                        out!(false, "critic hit its step cap after ignoring the required final verdict twice - fail-closed".to_string());
                    }
                    let tcs: Vec<Value> = calls.iter().map(|(id, cmd)| json!({
                        "id": id, "type": "function",
                        "function": {"name": TERM_EXEC_TOOL, "arguments": json!({"command": cmd}).to_string()}
                    })).collect();
                    messages.push(json!({"role": "assistant", "content": null, "tool_calls": tcs}));
                    for (id, cmd) in &calls {
                        trace.push(json!({"kind": "final_tool_rejected", "command": cmd}));
                        messages.push(json!({"role": "tool", "tool_call_id": id,
                            "content": "NOT EXECUTED: investigation is closed; return the required final verdict JSON."}));
                    }
                    messages.push(json!({"role": "user", "content":
                        "Your tool calls were not executed because investigation is closed. FINAL VERDICT NOW. Reply with EXACTLY one JSON object and nothing else: {\"refuted\": true, \"reason\": \"<reproduced failure>\"} or {\"refuted\": false, \"reason\": \"<what you tested and re-derived>\"}."
                    }));
                    continue;
                }
                probes += calls.len() as u32;
                let tcs: Vec<Value> = calls
                    .iter()
                    .map(|(id, cmd)| json!({
                        "id": id, "type": "function",
                        "function": {"name": TERM_EXEC_TOOL, "arguments": json!({"command": cmd}).to_string()}
                    }))
                    .collect();
                messages.push(json!({"role": "assistant", "content": null, "tool_calls": tcs}));
                for (id, cmd) in &calls {
                    trace.push(json!({"kind": "term_exec", "command": cmd}));
                    let o = crate::termexec::run_readonly(workdir, cmd, cfg.cmd_timeout_secs);
                    let result_text = tail(
                        &format!(
                            "exit {}\nstdout:\n{}\nstderr:\n{}",
                            o["exit_code"].as_i64().map_or_else(|| "?".into(), |c| c.to_string()),
                            o["stdout"].as_str().unwrap_or(""),
                            o["stderr"].as_str().unwrap_or("")
                        ),
                        6000,
                    );
                    trace.push(json!({"kind": "term_result", "command": cmd, "output": result_text}));
                    messages.push(json!({"role": "tool", "tool_call_id": id, "content": result_text}));
                }
            }
        }
    }
}

/// The checker.run gate: phase 1 the author's declared checks, phase 2 the
/// independent critic. Phase 2 runs only when phase 1 is green.
#[must_use]
pub fn checker_gate(ws: &Path) -> Value {
    let phase1 = crate::selfcheck::check(ws);
    if phase1["passed"].as_bool() != Some(true) {
        return phase1;
    }
    // The tb rig hands the instruction over via HS_TB_INSTRUCTION_FILE;
    // the TUI session (long-lived plugin processes, env set at spawn)
    // writes it per mission to <ws>/.hs/instruction.txt (repl run_goal).
    let instruction = std::env::var("HS_TB_INSTRUCTION_FILE")
        .ok()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .filter(|s| !s.trim().is_empty())
        .or_else(|| std::fs::read_to_string(ws.join(".hs/instruction.txt")).ok())
        .unwrap_or_default();
    if instruction.trim().is_empty() {
        return json!({"passed": false, "error": "critic gate: no instruction anchor (HS_TB_INSTRUCTION_FILE unreadable and <ws>/.hs/instruction.txt missing) - fail-closed"});
    }
    // The author's own answer summary (when present) is part of what the
    // critic reviews: its claims are refutation targets.
    let mut brief = instruction;
    if let Ok(f) = std::env::var("HS_TB_ANSWER_FILE")
        && let Ok(a) = std::fs::read_to_string(&f)
            && !a.trim().is_empty() {
                brief.push_str("\n\nAUTHOR'S SUBMISSION SUMMARY:\n");
                brief.push_str(&a);
            }
    let checks_text = std::fs::read_to_string(ws.join(crate::selfcheck::CHECKS_REL)).unwrap_or_default();
    let cfg = RefuteConfig::from_env();
    let mut model: Box<dyn CriticModel> = match ScriptedCritic::from_env() {
        Some(s) => Box::new(s),
        None => match ProviderCritic::from_env() {
            Ok(d) => Box::new(d),
            Err(e) => {
                return json!({"passed": false, "error": format!("critic gate: no critic model available: {e} - fail-closed")});
            }
        },
    };
    let outcome = refute(ws, &brief, &checks_text, &cfg, model.as_mut());
    if let Ok(tp) = std::env::var("HS_CRITIC_TRACE") {
        let mut s = String::new();
        for t in &outcome.trace {
            s.push_str(&t.to_string());
            s.push('\n');
        }
        let _ = std::fs::write(&tp, s);
    }
    if outcome.passed {
        json!({
            "passed": true, "error": "",
            "critic": {"steps": outcome.steps, "cost_micros": outcome.cost_micros, "reason": outcome.reason},
        })
    } else {
        json!({"passed": false, "error": format!("independent critic: {}", outcome.reason)})
    }
}

// ---------------------------------------------------------------------------
// Scripted critic: test seam (unit tests + HS_CRITIC_SCRIPT integration).

pub struct ScriptedCritic {
    replies: std::collections::VecDeque<CriticReply>,
    err: Option<String>,
    seen: Vec<String>,
    seen_count: usize,
}

impl ScriptedCritic {
    #[must_use]
    pub fn new(replies: Vec<CriticReply>) -> Self {
        Self { replies: replies.into(), err: None, seen: vec![], seen_count: 0 }
    }
    #[must_use]
    pub fn failing(err: &str) -> Self {
        Self { replies: Default::default(), err: Some(err.to_string()), seen: vec![], seen_count: 0 }
    }
    #[must_use]
    pub fn seen_tool_results(&self) -> &Vec<String> {
        &self.seen
    }
    /// `HS_CRITIC_SCRIPT`: "|"-separated segments, each "tool:<cmd>",
    /// "refute:<reason>", or "clean". The zero-network critic for the
    /// README offline trial (and the test seam): the probe commands run
    /// for real against the candidate workdir. Live missions leave it
    /// unset so the real HS_CRITIC_MODEL critic (deepseek/glm) serves.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let s = std::env::var("HS_CRITIC_SCRIPT").ok()?;
        let mut replies = vec![];
        for seg in s.split('|') {
            if let Some(cmd) = seg.strip_prefix("tool:") {
                let id = format!("c{}", replies.len() + 1);
                replies.push(CriticReply::ToolCalls(vec![(id, cmd.to_string())]));
            } else if let Some(reason) = seg.strip_prefix("refute:") {
                replies.push(CriticReply::Final(
                    json!({"refuted": true, "reason": reason}).to_string(),
                ));
            } else if seg == "clean" {
                replies.push(CriticReply::Final(
                    json!({"refuted": false, "reason": "scripted clean"}).to_string(),
                ));
            }
        }
        Some(Self::new(replies))
    }
}

impl CriticModel for ScriptedCritic {
    fn step(&mut self, messages: &[Value]) -> Result<CriticReply, String> {
        for m in messages.iter().filter(|m| m["role"] == "tool") {
            let c = m["content"].as_str().unwrap_or("").to_string();
            if !self.seen.contains(&c) {
                self.seen.push(c);
            }
            self.seen_count += 1;
        }
        if let Some(e) = &self.err {
            return Err(e.clone());
        }
        Ok(self
            .replies
            .pop_front()
            .unwrap_or_else(|| CriticReply::Final("script exhausted without a verdict".into())))
    }
}

/// Which model family the critic runs on (Eric 2026-09-07: cross-model
/// critic - same-model author/critic pairs share interpretation errors).
/// Default deepseek; "glm" selects the z.ai provider. Unknown names are an
/// error: the gate fails closed, never a silent fallback.
pub fn provider_for(name: &str) -> Result<crate::realmodel::Provider, String> {
    match name {
        "deepseek" => Ok(crate::realmodel::deepseek()),
        "glm" => Ok(crate::realmodel::glm()),
        other => crate::realmodel::provider_by_name(other),
    }
}

pub fn provider_from_env() -> Result<crate::realmodel::Provider, String> {
    let name = std::env::var("HS_CRITIC_MODEL").unwrap_or_else(|_| "deepseek".into());
    provider_for(&name)
}

// ---------------------------------------------------------------------------
// Provider critic: production model. Minimal OpenAI-shaped client with the
// realmodel watchdog pattern (thread + recv_timeout: ureq's global timeout
// does not fire on a stalled body read). Key material is fill-only from the
// environment and never logged.

pub struct ProviderCritic {
    name: String,
    url: String,
    model: String,
    key: String,
    input_tokens: u64,
    output_tokens: u64,
    cost_micros: u64,
    in_micros: f64,
    cached_micros: f64,
    out_micros: f64,
}

impl ProviderCritic {
    pub fn from_env() -> Result<Self, String> {
        Self::for_provider(provider_from_env()?)
    }

    pub fn for_provider(p: crate::realmodel::Provider) -> Result<Self, String> {
        // Beat 6: key resolution mirrors the operator model's
        // (realmodel::load_key): env var, key-file env, then the
        // `hairspring setup` config-dir file - pre-fix the critic
        // read only the two env vars, so a fresh shell after guided
        // setup fail-closed on EVERY answer.submit (live proof:
        // caprun7 stream, 2026-09-10).
        let key = crate::realmodel::load_key(&p)?;
        let envf = |k: &str, d: f64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
        Ok(Self {
            name: p.name.clone(),
            url: std::env::var(&p.base_url_env).unwrap_or(p.default_base_url),
            model: std::env::var(&p.model_env).unwrap_or(p.default_model),
            key,
            input_tokens: 0,
            output_tokens: 0,
            cost_micros: 0,
            in_micros: envf(&p.price_in_env, p.default_in_micros),
            cached_micros: envf(&p.price_cached_env, p.default_cached_micros),
            out_micros: envf(&p.price_out_env, p.default_out_micros),
        })
    }

    fn attempt(&self, body: &Value) -> Result<Value, String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(crate::realmodel::watchdog_secs() + 30)))
            .build()
            .into();
        let mut resp = agent
            .post(&self.url)
            .header("Authorization", &format!("Bearer {}", self.key))
            .header("Content-Type", "application/json")
            .send_json(body)
            .map_err(|e| format!("{} critic: request failed: {e}", self.name))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("{} critic: HTTP {}", self.name, status.as_u16()));
        }
        resp.body_mut()
            .read_json()
            .map_err(|e| format!("{} critic: unparsable response: {e}", self.name))
    }
}

impl CriticModel for ProviderCritic {
    fn step(&mut self, messages: &[Value]) -> Result<CriticReply, String> {
        let tools = json!([{
            "type": "function",
            "function": {
                "name": TERM_EXEC_TOOL,
                "description": "Run a shell command on the live machine (cwd = task workdir, root, state persists). Read-only on deliverables; scratch in /tmp.",
                "parameters": {
                    "type": "object",
                    "properties": {"command": {"type": "string"}},
                    "required": ["command"]
                }
            }
        }]);
        let body = json!({
            "model": self.model,
            "temperature": 0,
            "messages": messages,
            "tools": tools,
            "tool_choice": "auto",
        });
        // Watchdog: a stalled provider must cost the gate a failure, not
        // hang the harness (realmodel finding 2026-09-03).
        let (tx, rx) = std::sync::mpsc::channel();
        let me = Self {
            name: self.name.clone(),
            url: self.url.clone(),
            model: self.model.clone(),
            key: self.key.clone(),
            input_tokens: 0,
            output_tokens: 0,
            cost_micros: 0,
            in_micros: self.in_micros,
            cached_micros: self.cached_micros,
            out_micros: self.out_micros,
        };
        let body2 = body.clone();
        std::thread::spawn(move || {
            let _ = tx.send(me.attempt(&body2));
        });
        let v = match rx.recv_timeout(std::time::Duration::from_secs(crate::realmodel::watchdog_secs())) {
            Ok(r) => r?,
            Err(_) => return Err(format!("{} critic: provider watchdog timeout", self.name)),
        };
        let usage = &v["usage"];
        let in_tok = usage["prompt_tokens"].as_u64().unwrap_or(0);
        let cached = usage["prompt_cache_hit_tokens"].as_u64().unwrap_or(0);
        let out_tok = usage["completion_tokens"].as_u64().unwrap_or(0);
        self.input_tokens += in_tok;
        self.output_tokens += out_tok;
        self.cost_micros += ((in_tok.saturating_sub(cached)) as f64 * self.in_micros
            + cached as f64 * self.cached_micros
            + out_tok as f64 * self.out_micros) as u64;
        let msg = &v["choices"][0]["message"];
        if let Some(tcs) = msg["tool_calls"].as_array()
            && !tcs.is_empty() {
                let mut calls = vec![];
                for tc in tcs {
                    let id = tc["id"].as_str().unwrap_or("c0").to_string();
                    let name = tc["function"]["name"].as_str().unwrap_or("");
                    if name != TERM_EXEC_TOOL {
                        return Err(format!("{} critic: unknown tool {name:?}", self.name));
                    }
                    let args: Value = serde_json::from_str(
                        tc["function"]["arguments"].as_str().unwrap_or("{}"),
                    )
                    .map_err(|e| format!("{} critic: bad tool arguments: {e}", self.name))?;
                    let cmd = args["command"]
                        .as_str()
                        .ok_or_else(|| format!("{} critic: tool call missing command", self.name))?
                        .to_string();
                    calls.push((id, cmd));
                }
                return Ok(CriticReply::ToolCalls(calls));
            }
        let content = msg["content"]
            .as_str()
            .ok_or_else(|| format!("{} critic: completion carried neither tool calls nor content", self.name))?;
        Ok(CriticReply::Final(content.to_string()))
    }

    fn usage(&self) -> (u64, u64, u64) {
        (self.input_tokens, self.output_tokens, self.cost_micros)
    }
}
