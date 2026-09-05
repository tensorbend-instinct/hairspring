//! HAIRSPRING gate 3 - the inner loop with semantic feedback (spec 6).
//!
//! One step: observe -> drain_feedback -> assemble -> model.call ->
//! validate -> submit -> checker verdict. The verdict is recorded as a
//! feedback event in BOTH ablation arms; in the ON arm it is also injected
//! into the next step's context (recorded as context_inject: what entered
//! the window, and why). Feedback never costs a model round trip.

pub mod realmodel;
pub mod mcpbridge;
pub mod assembler;
pub mod editapply;
pub mod evolve;
pub mod goal;
pub mod ledger;
pub mod repexec;
pub mod sweprompt;
pub mod repotools;

use hs_core::{EventBuilder, EventKind, Payload};
use hs_kernel::{Kernel, KernelError};
use hs_log::{LogError, StreamWriter};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum LoopError {
    Kernel(KernelError),
    Log(LogError),
    ModelOutput(String),
    Io(std::io::Error),
}
impl From<std::io::Error> for LoopError {
    fn from(e: std::io::Error) -> Self {
        LoopError::Io(e)
    }
}
impl From<KernelError> for LoopError {
    fn from(e: KernelError) -> Self {
        LoopError::Kernel(e)
    }
}
impl From<LogError> for LoopError {
    fn from(e: LogError) -> Self {
        LoopError::Log(e)
    }
}
impl std::fmt::Display for LoopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Kernel(e) => write!(f, "kernel: {e}"),
            Self::Log(e) => write!(f, "log: {e}"),
            Self::ModelOutput(e) => write!(f, "model output: {e}"),
            Self::Io(e) => write!(f, "io: {e}"),
        }
    }
}
impl std::error::Error for LoopError {}

#[derive(Debug)]
pub struct MissionResult {
    pub passed: bool,
    pub steps: u32,
    pub model_calls: u32,
    pub stream_id: uuid::Uuid,
    pub answer_path: PathBuf,
    /// True when the run was killed for exceeding its USD budget (gate 8:
    /// budget-killed missions score as failures, never as passes).
    pub budget_killed: bool,
    /// Some(msg) when the mission aborted on a harness failure (phase 1:
    /// a plugin declared PluginDead by the supervisor). The message names
    /// the plugin and the real cause. Harness-aborted missions book their
    /// steps-so-far; they are infrastructure failures, not model failures.
    pub harness_error: Option<String>,
}

pub struct InnerLoop {
    kernel: Kernel,
    writer: StreamWriter,
    stream_id: uuid::Uuid,
    log_root: PathBuf,
    feedback_injection: bool,
    max_steps: u32,
    cost_total_micros: u64,
    budget_micros: Option<u64>,
    progress_path: Option<PathBuf>,
    ledger: ledger::Ledger,
    context_budget_chars: usize,
    memory_store: Option<Box<dyn hs_memory::MemoryStore>>,
    goal: Option<goal::GoalSpec>,
    dead_tools: std::collections::HashSet<String>,
    /// Fix 4: mission wall budget (secs) + start instant, for the per-step
    /// "T-minus" header. None = wall not tracked (old behavior).
    wall_secs: Option<u64>,
    mission_started: Option<std::time::Instant>,
}

/// D1/W2: input budget from the VERIFIED provider context, minus an
/// output/reasoning reserve. kimi-k3: 1M-token context per Moonshot's
/// platform docs (models-overview, verified 2026-09-05); 64k reserve for
/// K3's always-on reasoning + output. Unknown models fall back
/// conservatively; --context-budget-tokens overrides.
pub const DEFAULT_CONTEXT_BUDGET_TOKENS: usize = 983_040;

pub fn default_budget_for_model(model: &str) -> usize {
    match model {
        "kimi-k3" => 1_048_576 - 65_536,
        _ => 200_000, // conservative fallback until the provider limit is verified
    }
}

impl InnerLoop {
    pub fn new(
        kernel: Kernel,
        log_root: &Path,
        feedback_injection: bool,
        max_steps: u32,
    ) -> Result<Self, LoopError> {
        let stream_id = uuid::Uuid::new_v4();
        let writer = StreamWriter::create(log_root, stream_id)?;
        Ok(InnerLoop {
            kernel,
            writer,
            stream_id,
            log_root: log_root.to_path_buf(),
            feedback_injection,
            max_steps,
            cost_total_micros: 0,
            budget_micros: None,
            progress_path: None,
            ledger: Default::default(),
            context_budget_chars: DEFAULT_CONTEXT_BUDGET_TOKENS * 4,
            memory_store: None,
            goal: None,
            dead_tools: Default::default(),
            wall_secs: None,
            mission_started: None,
        })
    }

    /// Run on a pre-created stream (gate 5: a spawned child's stream is
    /// created and linked by the spawner, then adopted here).
    pub fn with_stream(
        kernel: Kernel,
        log_root: &Path,
        stream_id: uuid::Uuid,
        feedback_injection: bool,
        max_steps: u32,
    ) -> Result<Self, LoopError> {
        let outcome = StreamWriter::resume(log_root, stream_id)?;
        Ok(InnerLoop {
            kernel,
            writer: outcome.writer,
            stream_id,
            log_root: log_root.to_path_buf(),
            feedback_injection,
            max_steps,
            cost_total_micros: 0,
            budget_micros: None,
            progress_path: None,
            ledger: Default::default(),
            context_budget_chars: DEFAULT_CONTEXT_BUDGET_TOKENS * 4,
            memory_store: None,
            goal: None,
            dead_tools: Default::default(),
            wall_secs: None,
            mission_started: None,
        })
    }

    pub fn stream_id(&self) -> uuid::Uuid {
        self.stream_id
    }

    /// Cumulative real-model spend across this loop's missions (micro-USD),
    /// summed from the providers' own usage reports.
    pub fn total_cost_micros(&self) -> u64 {
        self.cost_total_micros
    }

    /// Hard per-mission USD budget (micro-USD). When cumulative provider-reported
    /// cost would exceed the cap, the mission is killed and scored as failed.
    pub fn set_budget_micros(&mut self, micros: u64) {
        self.budget_micros = Some(micros);
    }

    /// Fix 4 (ab2): the mission's wall budget in seconds. The runner enforces
    /// it externally; this makes it VISIBLE to the model every step.
    pub fn set_wall_secs(&mut self, secs: u64) {
        self.wall_secs = Some(secs);
    }

    /// Wall-kill resilience (phase 1, design D6): when set, the loop writes
    /// a JSON checkpoint of {steps, model_calls, cost_micros} after EVERY
    /// step. An external wall-clock kill (timeout, OOM, SIGKILL) then books
    /// from the checkpoint via `book_wall_kill` instead of writing a
    /// 0-step result for a run that did real work.
    /// D3: attach the typed memory plane; the assembler retrieves top-k
    /// records into every prompt (with source_seqs provenance).
    pub fn set_memory_db(&mut self, path: &Path) {
        self.memory_store = Some(Box::new(
            hs_memory::sqlite::SqliteMemoryStore::open(path).expect("memory db open"),
        ));
    }

    /// D6: acceptance-constrained stopping. The stop decision becomes a
    /// verifiable predicate (patch applies + F2P green in the sandbox).
    pub fn set_goal_evaluator(&mut self, ws: &Path, f2p: Vec<String>) {
        self.goal = Some(goal::GoalSpec { ws: ws.to_path_buf(), f2p, timeout_secs: 120 });
    }

    /// D1: size the transcript projection in tokens (4 chars/token proxy).
    pub fn set_context_budget_tokens(&mut self, tokens: usize) {
        self.context_budget_chars = tokens.saturating_mul(4);
    }

    pub fn set_progress_path(&mut self, path: &Path) {
        self.progress_path = Some(path.to_path_buf());
    }

    fn checkpoint(&self, steps: u32, model_calls: u32) {
        if let Some(p) = &self.progress_path {
            let body = serde_json::json!({
                "steps": steps,
                "model_calls": model_calls,
                "cost_micros": self.cost_total_micros,
            });
            let _ = std::fs::write(p, serde_json::to_string(&body).unwrap());
        }
    }

    /// Answer-path tools whose death is mission-terminal (measurement run
    /// ab2/17123): without edit.apply/answer.write no mission can land, so
    /// continuing burns steps for nothing. Death of any OTHER tool degrades
    /// to feedback instead of aborting - the mission continues while the
    /// answer path remains usable.
    fn is_answer_path(tool: &str) -> bool {
        matches!(tool, "answer.write" | "edit.apply")
    }

    /// Abort the mission on a supervisor-declared dead ANSWER-PATH plugin:
    /// book the harness_error as a Feedback event (trace-visible) and return
    /// the partial result. This replaces the old behavior of feeding the
    /// error back and burning the remaining steps against a dead plugin (run
    /// 17117 lost ~24 calls that way).
    fn abort_harness(
        &mut self,
        mission: &str,
        answer_path: &Path,
        steps: u32,
        model_calls: u32,
        msg: String,
    ) -> Result<MissionResult, LoopError> {
        self.writer.append(
            EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                serde_json::to_vec(&serde_json::json!({
                    "harness_error": msg, "mission": mission, "steps": steps,
                }))
                .unwrap(),
            )),
        )?;
        Ok(MissionResult {
            passed: false,
            steps,
            model_calls,
            stream_id: self.stream_id,
            answer_path: answer_path.to_path_buf(),
            budget_killed: false,
            harness_error: Some(msg),
        })
    }

    /// Run one mission to a checker verdict, the step cap, or the budget cap.
    pub fn run_mission(&mut self, mission: &str) -> Result<MissionResult, LoopError> {
        self.run_mission_full(mission, mission)
    }

    /// Gate 8: a mission whose PROMPT differs from its id. The id names the
    /// work dir (must be path-safe); the prompt is the full mission text the
    /// model sees (e.g. a SWE-bench problem statement + response contract).
    pub fn run_mission_full(
        &mut self,
        mission_id: &str,
        prompt: &str,
    ) -> Result<MissionResult, LoopError> {
        let mission = mission_id;
        self.mission_started = Some(std::time::Instant::now());
        let answer_path = self.log_root.join("work").join(mission).join("answer.txt");
        std::fs::create_dir_all(answer_path.parent().unwrap())?;
        let mut pending_feedback: Vec<String> = vec![];
        let mut steps = 0u32;
        let mut model_calls = 0u32;

        for step in 1..=self.max_steps {
            steps = step;
            // observe + drain_feedback: what the world said since last step
            let artifact = std::fs::read_to_string(&answer_path).unwrap_or_default();
            let drained = std::mem::take(&mut pending_feedback);

            // assemble. KV-cache discipline (spec v4): the stable, append-only
            // sections lead - MISSION then TRANSCRIPT - so the cached prefix
            // grows monotonically; volatile lines (ATTEMPT/ARTIFACT/FEEDBACK)
            // go last, after the transcript tail.
            let mut ctx = format!("MISSION: {prompt}\n");
            // Fix 4: budget visibility every step - "step N of MAX, T-minus
            // Xs, $Y of $Z spent" (ab2: the model could not pace itself
            // because it never saw a budget).
            let mut volatile = format!("ATTEMPT: step {step} of {}", self.max_steps);
            if let (Some(w), Some(t0)) = (self.wall_secs, self.mission_started) {
                let rem = w.saturating_sub(t0.elapsed().as_secs());
                volatile.push_str(&format!(", T-minus {rem}s"));
            }
            if let Some(cap) = self.budget_micros {
                volatile.push_str(&format!(
                    ", ${:.2} of ${:.2} spent",
                    self.cost_total_micros as f64 / 1e6,
                    cap as f64 / 1e6
                ));
            }
            volatile.push_str(&format!(
                "\nANSWER_PATH: {}\nARTIFACT: {}\n",
                answer_path.display(),
                if artifact.is_empty() {
                    "<none>"
                } else {
                    artifact.trim()
                }
            ));
            let mut injected = false;
            if self.feedback_injection && !drained.is_empty() {
                volatile.push_str("FEEDBACK:\n");
                for f in &drained {
                    volatile.push_str(&format!("- {f}\n"));
                }
                injected = true;
            }
            // D2/D1: the LEDGER summary is always resident (bounded); the
            // transcript is a token-budgeted projection of the stream's own
            // ToolCall events. No parallel store: both are read models over
            // the log and survive restarts/freeze recovery.
            if self.feedback_injection {
                if let Ok(reader) = hs_log::StreamReader::open(&self.log_root, self.stream_id) {
                    if let Ok(events) = reader.events() {
                        ctx.push_str("LEDGER (your work so far, always current):\n");
                        ctx.push_str(&self.ledger.summary());
                        if let Some(store) = &self.memory_store {
                            if let Ok(recs) = store.top_k("operator", 5) {
                                if !recs.is_empty() {
                                    ctx.push_str("MEMORY (earlier missions):\n");
                                    let mut budget = 2000usize;
                                    for r in &recs {
                                        let line = format!(
                                            "- [{} seqs:{}] {}\n",
                                            r.kind,
                                            r.source_seqs.iter().map(u64::to_string).collect::<Vec<_>>().join(","),
                                            r.content
                                        );
                                        if line.len() > budget { break; }
                                        budget -= line.len();
                                        ctx.push_str(&line);
                                    }
                                }
                            }
                        }
                        let mut asm = assembler::assemble(&reader, &events, self.context_budget_chars);
                        if let Some(c) = &asm.compressed {
                            // D1: distill the oldest events into the Codex
                            // four-element handoff contract via the model;
                            // the call is booked with its cost. On any
                            // failure the ledger pointer already in place
                            // stays - compression never destroys content.
                            let distill_prompt = format!(
                                "DISTILL: You are compacting an agent's earlier tool-call history for a fresh context. Summarize the calls below into EXACTLY four labeled sections: PROGRESS AND DECISIONS / CONSTRAINTS AND PREFERENCES / NEXT STEPS / CRITICAL DATA. Be terse; preserve file paths, line numbers, test names, and verdicts.\n\n{}",
                                c.lines.join("\n")
                            );
                            let mut distilled: Option<String> = None;
                            match self.kernel.call_model("operator", None, &distill_prompt) {
                                Ok(out) => {
                                    model_calls += 1;
                                    self.cost_total_micros += out.cost_usd_micros.max(0) as u64;
                                    let _ = self.writer.append(
                                        EventBuilder::new(EventKind::ModelCall)
                                            .payload(Payload::Inline(
                                                serde_json::to_vec(&serde_json::json!({
                                                    "model": out.model, "why": "distill",
                                                    "prompt": distill_prompt, "completion": out.completion,
                                                    "input_tokens": out.input_tokens,
                                                    "output_tokens": out.output_tokens,
                                                    "cost_usd_micros": out.cost_usd_micros,
                                                }))
                                                .unwrap(),
                                            ))
                                            .latency_ms(out.latency_ms)
                                            .cost_usd_micros(out.cost_usd_micros),
                                    );
                                    distilled = Some(out.completion);
                                }
                                Err(_) => {}
                            }
                            let did_distill = distilled.is_some();
                            if let Some(summary) = distilled {
                                // Item 2 (Codex handoff framing): a colleague
                                // handed this work off - build on it, don't
                                // re-verify it from scratch.
                                asm.entries[0] = format!(
                                    "COMPACTED {} earlier tool calls (events seq {}..{}, refs {}..{}). Another run started this mission and did that work before handing off to you. Its handoff summary follows - build on it, do not redo it:\n{}",
                                    c.count, c.lo_seq, c.hi_seq, c.lo_id, c.hi_id, summary
                                );
                            }
                            let _ = self.writer.append(
                                EventBuilder::new(EventKind::ContextInject)
                                    .payload(Payload::Inline(
                                        format!(
                                            "context_inject why=pressure compacted={} range=seq{}..seq{} distilled={}",
                                            c.count, c.lo_seq, c.hi_seq, did_distill
                                        )
                                        .into_bytes(),
                                    )),
                            );
                        }
                        if !asm.entries.is_empty() {
                            ctx.push_str("TRANSCRIPT (earlier tool calls):\n");
                            for e in &asm.entries {
                                ctx.push_str(&format!("- {e}\n"));
                            }
                        }
                    }
                }
            }

            ctx.push_str(&volatile);

            // the only model round trip in the step
            let out = match self.kernel.call_model("operator", None, &ctx) {
                Ok(o) => o,
                Err(e @ KernelError::PluginApp { .. }) => {
                    // persistent provider failure (the plugin already burned
                    // its own retries): book it, don't strike-loop
                    self.checkpoint(steps, model_calls);
                    return self.abort_harness(mission, &answer_path, steps, model_calls, e.to_string());
                }
                Err(e @ KernelError::PluginDead { .. }) => {
                    self.checkpoint(steps, model_calls);
                    return self.abort_harness(
                        mission,
                        &answer_path,
                        steps,
                        model_calls,
                        e.to_string(),
                    );
                }
                Err(e) => return Err(e.into()),
            };
            model_calls += 1;
            self.cost_total_micros += out.cost_usd_micros.max(0) as u64;
            // T5c: checkpoint EVERY step after the model-call accounting,
            // answer or not - a wall kill must never book a 0-step row for
            // a mission that did real work (ab2 17092/17102/17117 lost
            // 19-25 steps each to answer-only checkpointing).
            self.checkpoint(steps, model_calls);
            if let Some(cap) = self.budget_micros {
                if self.cost_total_micros > cap {
                    self.writer.append(
                        EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                            serde_json::to_vec(&serde_json::json!({
                                "budget_killed": true, "cap_micros": cap,
                                "cost_micros": self.cost_total_micros,
                            }))
                            .unwrap(),
                        )),
                    )?;
                    self.checkpoint(steps, model_calls);
                    return Ok(MissionResult {
                        passed: false,
                        steps,
                        model_calls,
                        stream_id: self.stream_id,
                        answer_path,
                        budget_killed: true,
                        harness_error: None,
                    });
                }
            }
            self.writer.append(
                EventBuilder::new(EventKind::ModelCall)
                    .payload(Payload::Inline(
                        serde_json::to_vec(&serde_json::json!({
                            "model": out.model, "prompt": ctx, "completion": out.completion,
                            "input_tokens": out.input_tokens, "output_tokens": out.output_tokens,
                        }))
                        .unwrap(),
                    ))
                    .latency_ms(out.latency_ms)
                    .cost_usd_micros(out.cost_usd_micros),
            )?;
            if injected {
                self.writer.append(
                    EventBuilder::new(EventKind::ContextInject).payload(Payload::Inline(
                        serde_json::to_vec(&serde_json::json!({
                            "what": drained, "why": "checker verdict since last step",
                        }))
                        .unwrap(),
                    )),
                )?;
            }

            // validate: the plan must be a single well-formed action. A real
            // model's malformed output is not a harness failure: record it as
            // a tool error and feed it back on the next step.
            let plan: Result<serde_json::Value, _> = serde_json::from_str(&out.completion);
            let validated = plan.ok().and_then(|plan| {
                let tool = plan["tool"].as_str()?.to_string();
                Some((tool, plan["args"].clone()))
            });

            // submit (tool errors are feedback too: models produce bad args)
            let mut wrote_answer = false;
            let tool_feedback = match validated {
                None => Some(r#"your reply was not a single JSON tool call; respond with exactly {"tool":"answer.write","args":{"path":<ANSWER_PATH>,"content":...}}"#.to_string()),
                Some((tool, args)) if self.dead_tools.contains(&tool) => {
                    // T3c: dead tools short-circuit - feedback, no respawn
                    let msg = format!(
                        "tool {tool} is dead for the rest of this mission - pick another tool (the answer path, edit.apply/answer.write, is intact)"
                    );
                    self.writer.append(
                        EventBuilder::new(EventKind::ToolCall).payload(Payload::Inline(
                            serde_json::to_vec(&serde_json::json!({
                                "plugin": tool, "args": args, "error": msg,
                            }))
                            .unwrap(),
                        )),
                    )?;
                    Some(msg)
                }
                Some((tool, args)) => match self.kernel.call_tool("operator", &tool, args.clone()) {
                    Ok(tool_out) => {
                        let ev = self.writer.append(
                            EventBuilder::new(EventKind::ToolCall)
                                .payload(Payload::Inline(
                                    serde_json::to_vec(&serde_json::json!({
                                        "plugin": tool, "args": args, "result": tool_out.output,
                                    }))
                                    .unwrap(),
                                ))
                                .latency_ms(tool_out.latency_ms),
                        )?;
                        // D2: exact duplicate (tool, args) calls get flagged
                        // with the prior seq - an explicit, correctable
                        // signal instead of a silent re-read loop (P3)
                        if let Some(prior) = self.ledger.find_duplicate(&tool, &args) {
                            let note = format!(
                                "duplicate call: identical {tool} args already served at seq {prior} - that result is in your transcript/ledger; do not re-run it"
                            );
                            self.writer.append(
                                EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                                    serde_json::to_vec(&serde_json::json!({
                                        "duplicate_call": tool, "prior_seq": prior, "note": note,
                                    }))
                                    .unwrap(),
                                )),
                            )?;
                            pending_feedback.push(note);
                        }
                        self.ledger.apply_tool_call(ev.seq, &tool, &args, &tool_out.output);
                        if tool == "answer.write" {
                            wrote_answer = true;
                        }
                        // non-write results reach future steps via the
                        // log-sourced TRANSCRIPT above (read back from this
                        // stream's own ToolCall events), not a side channel
                        None
                    }
                    Err(KernelError::PluginDead { name, strikes, detail }) => {
                        // ab2/17123: death of a NON-answer tool degrades to
                        // feedback instead of aborting - the mission
                        // continues while the answer path remains usable.
                        // Only answer-path death is terminal.
                        if Self::is_answer_path(&name) {
                            self.checkpoint(steps, model_calls);
                            return self.abort_harness(
                                mission,
                                &answer_path,
                                steps,
                                model_calls,
                                KernelError::PluginDead { name, strikes, detail }.to_string(),
                            );
                        }
                        self.dead_tools.insert(name.clone());
                        let msg = format!(
                            "tool {name} is permanently unavailable (dead after {strikes} strikes: {detail}) - continue with the remaining tools; the answer path (edit.apply, answer.write) is intact"
                        );
                        self.writer.append(
                            EventBuilder::new(EventKind::ToolCall).payload(Payload::Inline(
                                serde_json::to_vec(&serde_json::json!({
                                    "plugin": name, "error": msg,
                                }))
                                .unwrap(),
                            )),
                        )?;
                        Some(msg)
                    }
                    Err(e) => {
                        let msg = format!("tool {tool} failed: {e}");
                        self.writer.append(
                            EventBuilder::new(EventKind::ToolCall)
                                .payload(Payload::Inline(
                                    serde_json::to_vec(&serde_json::json!({
                                        "plugin": tool, "args": args, "error": msg,
                                    }))
                                    .unwrap(),
                                )),
                        )?;
                        Some(msg)
                    }
                },
            };
            if let Some(msg) = tool_feedback {
                pending_feedback.push(format!("harness: {msg}"));
                continue;
            }
            if !wrote_answer {
                continue;
            }

            // the world answers (checker = ground truth at this gate);
            // only an answer.write produces something to judge
            let verdict = self.kernel.call_tool(
                "operator",
                "checker.run",
                serde_json::json!({"task_id": mission, "path": answer_path}),
            )?;
            let passed = verdict.output["passed"].as_bool().unwrap_or(false);
            let error = verdict.output["error"].as_str().unwrap_or("").to_string();
            // D6: when a goal evaluator is set, IT owns the stop decision -
            // a checker verdict (or a lying checker) cannot stop a red mission
            let stop_green = match &self.goal {
                Some(g) => {
                    let green = goal::verify(g, &answer_path);
                    self.writer.append(
                        EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                            serde_json::to_vec(&serde_json::json!({
                                "goal_evaluator": if green { "green" } else { "red" },
                                "f2p": g.f2p, "checker_passed": passed,
                            }))
                            .unwrap(),
                        )),
                    )?;
                    green
                }
                None => passed,
            };
            self.writer.append(
                EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                    serde_json::to_vec(&serde_json::json!({
                        "checker": "checker.run", "task_id": mission,
                        "passed": passed, "error": error,
                    }))
                    .unwrap(),
                )),
            )?;
            if stop_green {
                self.writer.append(
                    EventBuilder::new(EventKind::GoalUpdate).payload(Payload::Inline(
                        serde_json::to_vec(&serde_json::json!({"mission": mission, "done": true}))
                            .unwrap(),
                    )),
                )?;
                self.checkpoint(steps, model_calls);
                return Ok(MissionResult {
                    passed: true,
                    steps,
                    model_calls,
                    stream_id: self.stream_id,
                    answer_path,
                    budget_killed: false,
                    harness_error: None,
                });
            }
            if passed && !stop_green {
                pending_feedback.push(
                    "checker reported pass but the GOAL EVALUATOR is red: F2P still failing in the sandbox - keep working".to_string()
                );
            } else {
                pending_feedback.push(error);
            }
            self.checkpoint(steps, model_calls);
        }
        Ok(MissionResult {
            passed: false,
            steps,
            model_calls,
            stream_id: self.stream_id,
            answer_path,
            budget_killed: false,
            harness_error: None,
        })
    }
}


/// Book a wall-clock kill from a loop checkpoint (phase 1, T5): the run
/// did real work up to `steps`, so the result row must carry it - the old
/// runner wrote steps:0 on timeout, which both hid progress and poisoned
/// per-step cost accounting.
pub fn book_wall_kill(
    progress_path: &Path,
    instance_id: &str,
    model: &str,
    feedback: bool,
) -> serde_json::Value {
    let v: serde_json::Value = std::fs::read_to_string(progress_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({"steps": 0, "model_calls": 0, "cost_micros": 0}));
    serde_json::json!({
        "instance_id": instance_id,
        "model": model,
        "feedback": feedback,
        "passed": false,
        "steps": v["steps"].as_u64().unwrap_or(0),
        "model_calls": v["model_calls"].as_u64().unwrap_or(0),
        "cost_micros": v["cost_micros"].as_u64().unwrap_or(0),
        "budget_killed": false,
        "outcome": "wall_killed",
    })
}
