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
pub mod msgfmt;
pub mod editapply;
pub mod evolve;
pub mod goal;
pub mod ledger;
pub mod repexec;
pub mod sweprompt;
pub mod repotools;
pub mod toolschema;
pub mod verifier;

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
    /// item 4: per-plugin count at which the doom-loop nudge last fired;
    /// refires only when the repeat count grows by +2 (anti-spam)
    doom_nudges: std::collections::HashMap<String, usize>,
    /// post-B8: per-class guardrail fire counts; same-class repeats
    /// escalate (the bare refusal never landed with B8's model)
    guardrail_escalator: repexec::GuardrailEscalator,
    /// item 3: adversarial verifier state - rounds spent and the findings
    /// the last refuted round handed back (the next round's PRIOR_GAPS)
    verifier_rounds: u32,
    prior_gaps: Vec<String>,
    /// Fix 4: mission wall budget (secs) + start instant, for the per-step
    /// "T-minus" header. None = wall not tracked (old behavior).
    wall_secs: Option<u64>,
    mission_started: Option<std::time::Instant>,
    /// Native tool schemas delivered to the provider's tools parameter on
    /// the operator call (native tool calling; Eric 2026-09-05). None = the
    /// model gets no tools param (legacy/text missions, unit fixtures).
    tools: Option<serde_json::Value>,
    /// Gate-8 async verifier seam (waste-only redesign, Eric 2026-09-06):
    /// when on, a checker-green submit snapshots the ws, fires the audit on
    /// a worker thread, and the agent keeps working; a decided verdict
    /// restores the audited snapshot, so the ws is ALWAYS exactly what the
    /// verifier judged. Verdict inputs are frozen at submit - the verdict
    /// is bit-identical to the synchronous path.
    async_verify: bool,
    ws_path: Option<PathBuf>,
    world: Option<hs_world::World>,
    pending_verdict: Option<verifier::PendingVerdict>,
    verdict_cache: std::collections::HashMap<String, verifier::RecordedVerdict>,
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
            tools: None,
            progress_path: None,
            ledger: Default::default(),
            context_budget_chars: DEFAULT_CONTEXT_BUDGET_TOKENS * 4,
            memory_store: None,
            goal: None,
            dead_tools: Default::default(),
            doom_nudges: Default::default(),
            guardrail_escalator: Default::default(),
            verifier_rounds: 0,
            prior_gaps: vec![],
            wall_secs: None,
            mission_started: None,
            async_verify: false,
            ws_path: None,
            world: None,
            pending_verdict: None,
            verdict_cache: Default::default(),
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
            tools: None,
            progress_path: None,
            ledger: Default::default(),
            context_budget_chars: DEFAULT_CONTEXT_BUDGET_TOKENS * 4,
            memory_store: None,
            goal: None,
            dead_tools: Default::default(),
            doom_nudges: Default::default(),
            guardrail_escalator: Default::default(),
            verifier_rounds: 0,
            prior_gaps: vec![],
            wall_secs: None,
            mission_started: None,
            async_verify: false,
            ws_path: None,
            world: None,
            pending_verdict: None,
            verdict_cache: Default::default(),
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

    /// Native tool schemas for the operator model call (builtin +
    /// MCP-discovered), delivered via the provider API's tools parameter.
    pub fn set_tools(&mut self, tools: serde_json::Value) {
        self.tools = Some(tools);
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
    /// ab2/17123): without edit.patch/answer.submit no mission can land, so
    /// continuing burns steps for nothing. Death of any OTHER tool degrades
    /// to feedback instead of aborting - the mission continues while the
    /// answer path remains usable.
    fn is_answer_path(tool: &str) -> bool {
        matches!(tool, "answer.submit" | "edit.patch" | "edit.anchor" | "answer.write" | "edit.apply")
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
            // gate-8 async seam: a verdict that landed since the last step
            // is processed BEFORE assembly, so its feedback enters this
            // step's context and a banked mission ends here.
            if self.pending_verdict.is_some() {
                if let Some(r) = self.process_verdict(
                    false,
                    mission,
                    &answer_path,
                    steps,
                    &mut model_calls,
                    &mut pending_feedback,
                )? {
                    return Ok(r);
                }
            }
            // observe + drain_feedback: what the world said since last step
            let artifact = std::fs::read_to_string(&answer_path).unwrap_or_default();
            let drained = std::mem::take(&mut pending_feedback);

            // assemble. KV-cache discipline (spec v4): the stable, append-only
            // sections lead - MISSION then TRANSCRIPT - so the cached prefix
            // grows monotonically; volatile lines (ATTEMPT/ARTIFACT/FEEDBACK)
            // go last, after the transcript tail.
            let t_assembly = std::time::Instant::now(); // time audit (Eric 2026-09-05)
            // Structured messages (user directive 2026-09-05: EVERYTHING
            // native, transcript included - efficiency first). The array is
            // append-only: [mission][compaction?][history pairs...][state
            // tail]. Every mutable block (ATTEMPT budget, ANSWER_PATH,
            // ARTIFACT, FEEDBACK, LEDGER, MEMORY) rides ONLY in the final
            // tail message, so the provider's cached prefix grows
            // monotonically and no prior message is ever rewritten between
            // steps (pre-migration the mutating LEDGER sat BEFORE the
            // transcript, busting the cache for the whole history).
            let mut messages: Vec<serde_json::Value> = vec![serde_json::json!({
                "role": "user",
                "content": format!("MISSION: {prompt}"),
            })];
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
            let mut injected = false;
            if self.feedback_injection && !drained.is_empty() {
                volatile.push_str("FEEDBACK:\n");
                for f in &drained {
                    volatile.push_str(&format!("- {f}\n"));
                }
                injected = true;
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
            // Fix 5: convergence pressure (ab2: three wall-killed missions
            // ran 19-25 steps with zero model-initiated verification). At
            // 50% and 75% of the step budget, when the model has never run
            // a test itself, the header says so in plain terms.
            let half = self.max_steps.div_ceil(2);
            let three_q = (self.max_steps * 3).div_ceil(4);
            let convergence_note = if self.feedback_injection
                && (step == half || step == three_q)
                && !self.ledger.model_verified()
            {
                let note = format!(
                    "CONVERGENCE: step {step} of {} and you have not run a test or check yourself. Verify your current hypothesis NOW (run a test, a build, or a checker), or state in one line what you will change and how you will verify it.",
                    self.max_steps
                );
                volatile.push_str(&note);
                volatile.push('\n');
                Some(note)
            } else {
                None
            };
            // D2/D1: the LEDGER summary is always resident (bounded); the
            // history is a token-budgeted projection of the stream's own
            // ToolCall events, replayed as native assistant/tool pairs. No
            // parallel store: both are read models over the log and survive
            // restarts/freeze recovery.
            if self.feedback_injection {
                if let Ok(reader) = hs_log::StreamReader::open(&self.log_root, self.stream_id) {
                    if let Ok(events) = reader.events() {
                        volatile.push_str("LEDGER (your work so far, always current):\n");
                        volatile.push_str(&self.ledger.summary());
                        if let Some(store) = &self.memory_store {
                            if let Ok(recs) = store.top_k("operator", 5) {
                                if !recs.is_empty() {
                                    volatile.push_str("MEMORY (earlier missions):\n");
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
                                        volatile.push_str(&line);
                                    }
                                }
                            }
                        }
                        let mut asm = assembler::assemble_messages(&reader, &events, self.context_budget_chars);
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
                                                    "reasoning_tokens": out.reasoning_tokens,
                                                    "reasoning_content": out.reasoning_content,
                                                    "cached_tokens": out.cached_tokens,
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
                                asm.messages[0] = serde_json::json!({
                                    "role": "user",
                                    "content": format!(
                                        "COMPACTED {} earlier tool calls (events seq {}..{}, refs {}..{}). Another run started this mission and did that work before handing off to you. Its handoff summary follows - build on it, do not redo it:\n{}",
                                        c.count, c.lo_seq, c.hi_seq, c.lo_id, c.hi_id, summary
                                    ),
                                });
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
                        messages.extend(asm.messages);
                    }
                }
            }

            messages.push(serde_json::json!({"role": "user", "content": volatile}));
            let assembly_ms = t_assembly.elapsed().as_millis() as u64; // capture BEFORE the model call (was after: read as ~latency)
            let messages = serde_json::Value::Array(messages);

            // the only model round trip in the step
            let out = match self.kernel.call_model_messages("operator", None, &messages, self.tools.as_ref()) {
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
                            "model": out.model, "messages": messages, "completion": out.completion,
                            "input_tokens": out.input_tokens, "output_tokens": out.output_tokens,
                            "reasoning_tokens": out.reasoning_tokens,
                            "reasoning_content": out.reasoning_content,
                            "cached_tokens": out.cached_tokens,
                            "assembly_ms": assembly_ms,
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
            if let Some(note) = convergence_note {
                self.writer.append(
                    EventBuilder::new(EventKind::ContextInject).payload(Payload::Inline(
                        serde_json::to_vec(&serde_json::json!({
                            "what": [note], "why": "convergence",
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
                None => Some("your reply carried no tool call; call exactly one of the provided tools (the answer path is answer.submit with the ANSWER_PATH) - no prose".to_string()),
                Some((tool, args)) if self.dead_tools.contains(&tool) => {
                    // T3c: dead tools short-circuit - feedback, no respawn
                    let msg = format!(
                        "tool {tool} is dead for the rest of this mission - pick another tool (the answer path, edit.patch/answer.submit, is intact)"
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
                // Hard rule (Eric, 2026-09-05): an untested answer.submit is
                // REJECTED with feedback whenever steps remain - submitting
                // without running the mission's own checks must never spend a
                // checker cycle. At the last step a hail-mary goes through:
                // an unverified answer beats no answer.
                Some((tool, args)) if (tool == "answer.submit" || tool == "answer.write")
                    && !self.ledger.model_verified()
                    && step < self.max_steps
                    // the rejection must name an action the model can
                    // actually take: no verification tool in this mission's
                    // config, no gate (checker-only rigs, probe missions)
                    && self.kernel.list_tools("operator").iter().any(|t| t.name == "repo.exec") =>
                {
                    let msg = format!(
                        "answer.submit REJECTED: no verification run yet. Run the mission's own checks first (repo.exec against your candidate diff, or the mission's stated test command) - a submission with zero test evidence is not a submission. Steps remaining: {}",
                        self.max_steps - step
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
                        // Post-B8: guardrail escalation - same-class
                        // edit-path violations are counted per class; from
                        // the second fire on, an escalating steer is
                        // injected (B8's model retried the forbidden class
                        // 6 times against the bare refusal).
                        if tool == "repo.exec" {
                            if let Some(class) = repexec::extract_gate_class(&tool_out.output.to_string()) {
                                if let Some(note) = self.guardrail_escalator.record(&class) {
                                    self.writer.append(
                                        EventBuilder::new(EventKind::ContextInject).payload(
                                            Payload::Inline(
                                                serde_json::to_vec(&serde_json::json!({
                                                    "what": [note.clone()], "why": "guardrail_escalation",
                                                }))
                                                .unwrap(),
                                            ),
                                        ),
                                    )?;
                                    pending_feedback.push(note);
                                }
                            }
                        }
                        // Item 4: doom-loop detection (Grok doom_loop_telemetry,
                        // adapted). Third effectively-identical call in the
                        // window triggers one recovery nudge; it refires only
                        // when the streak grows by another 2.
                        const DOOM_WINDOW: usize = 8;
                        const DOOM_THRESHOLD: usize = 3;
                        if let Some((plugin, count)) =
                            self.ledger.doom_loop_repeat(DOOM_WINDOW, DOOM_THRESHOLD)
                        {
                            let last = self.doom_nudges.get(&plugin).copied().unwrap_or(0);
                            if last == 0 || count >= last + 2 {
                                let note = if self.prior_gaps.is_empty() {
                                    format!(
                                        "DOOM LOOP: {plugin} with effectively the same args {count} times in the last {DOOM_WINDOW} calls - repeating it is not working. STOP re-issuing it: change one thing materially (a different command, a different file, a different hypothesis), or verify and submit."
                                    )
                                } else {
                                    format!(
                                        "DOOM LOOP: {plugin} with effectively the same args {count} times in the last {DOOM_WINDOW} calls - repeating it is not working. The verifier REFUTED your last submission: re-issuing this call repairs nothing. Change one thing materially (a different command, a different file, a different hypothesis) to address the findings in FEEDBACK."
                                    )
                                };
                                self.writer.append(
                                    EventBuilder::new(EventKind::ContextInject).payload(
                                        Payload::Inline(
                                            serde_json::to_vec(&serde_json::json!({
                                                "what": [note.clone()], "why": "doom_loop",
                                            }))
                                            .unwrap(),
                                        ),
                                    ),
                                )?;
                                pending_feedback.push(note);
                                self.doom_nudges.insert(plugin, count);
                            }
                        }
                        if tool == "answer.submit" || tool == "answer.write" {
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
                            "tool {name} is permanently unavailable (dead after {strikes} strikes: {detail}) - continue with the remaining tools; the answer path (edit.patch, answer.submit) is intact"
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
            // only an answer.submit produces something to judge
            let verdict = self.kernel.call_tool(
                "operator",
                "checker.run",
                serde_json::json!({"task_id": mission, "path": answer_path}),
            )?;
            let passed = verdict.output["passed"].as_bool().unwrap_or(false);
            let error = verdict.output["error"].as_str().unwrap_or("").to_string();
            // D6 (revised post-A7, FIXLIST 2026-09-05 item 1): a green
            // checker.run verdict ENDS the mission - the adversarial
            // verifier veto below still runs after it. A red goal evaluator
            // cannot hold a checker-passed mission to the budget/wall
            // guards: A7's evaluator re-ran f2p through the exec sandbox
            // (no pytest), went red environmentally, and vetoed the stop
            // for 22 steps / $1.46 after checker_passed:true. A goal-red +
            // checker-green conflict is recorded, never silently resolved.
            // Checker red + goal green still stops (goal owns that case).
            let stop_green = match &self.goal {
                Some(g) => {
                    let verdict = goal::verify_verdict(g, &answer_path);
                    let green = verdict == goal::GoalVerdict::Pass;
                    let env_limit = match &verdict {
                        goal::GoalVerdict::EnvLimited(r) => Some(r.clone()),
                        _ => None,
                    };
                    self.writer.append(
                        EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                            serde_json::to_vec(&serde_json::json!({
                                "goal_evaluator": if green { "green" } else if env_limit.is_some() { "env_limited" } else { "red" },
                                "env_limit": env_limit, "f2p": g.f2p, "checker_passed": passed,
                            }))
                            .unwrap(),
                        )),
                    )?;
                    if passed && !green {
                        self.writer.append(
                            EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                                serde_json::to_vec(&serde_json::json!({
                                    "conflict": "checker green overrides goal red",
                                    "detail": "goal evaluator vetoed a checker-passed mission - the checker verdict stands (post-A7 rule)",
                                }))
                                .unwrap(),
                            )),
                        )?;
                    }
                    green || passed
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
                match self.stop_green_tail(
                    mission,
                    &answer_path,
                    steps,
                    &mut model_calls,
                    &mut pending_feedback,
                )? {
                    StopGreen::Continue => {
                        self.checkpoint(steps, model_calls);
                        continue;
                    }
                    StopGreen::Banked(r) => return Ok(r),
                }
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

    /// Gate-8 async verify: enable the speculative-continuation seam for
    /// this loop. `ws` is the mission workspace - snapshotted at every
    /// checker-green submit; restored on bank AND on veto, so the ws (and
    /// the patch extracted from it) is always exactly the audited state.
    pub fn set_async_verify(&mut self, ws: &Path) {
        self.async_verify = true;
        self.ws_path = Some(ws.to_path_buf());
        self.world = Some(hs_world::World::open(&self.log_root).expect("world open"));
    }

    /// The stop_green tail: the adversarial verifier veto (item 3), in
    /// synchronous legacy mode or gate-8 async speculative mode. Rounds,
    /// prompts, authority, and the ratchet are identical across modes;
    /// only who waits for the audit differs.
    fn stop_green_tail(
        &mut self,
        mission: &str,
        answer_path: &Path,
        steps: u32,
        model_calls: &mut u32,
        pending_feedback: &mut Vec<String>,
    ) -> Result<StopGreen, LoopError> {
        if self.async_verify && self.pending_verdict.is_some() {
            // A fresh green submit while an audit is in flight. Rounds are
            // strictly sequential - round N+1's prompt carries round N's
            // gaps - so this submit waits for the pending verdict first.
            if let Some(r) = self.process_verdict(
                true,
                mission,
                answer_path,
                steps,
                model_calls,
                pending_feedback,
            )? {
                return Ok(StopGreen::Banked(r));
            }
            // refuted: the restore wiped THIS submission's ws state - the
            // agent repairs from the audited patch and resubmits. No new
            // round is fired here.
            return Ok(StopGreen::Continue);
        }
        if self.verifier_rounds < verifier::VERIFIER_MAX_ROUNDS {
            self.verifier_rounds += 1;
            let round = self.verifier_rounds;
            if self.async_verify {
                return self.async_verifier_round(
                    mission,
                    answer_path,
                    round,
                    steps,
                    model_calls,
                    pending_feedback,
                );
            }
            return self.sync_verifier_round(
                mission,
                answer_path,
                round,
                steps,
                model_calls,
                pending_feedback,
            );
        }
        self.writer.append(
            EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                serde_json::to_vec(&serde_json::json!({
                    "why": "verifier_ratchet", "rounds": verifier::VERIFIER_MAX_ROUNDS,
                    "detail": "verifier failed to converge in 3 rounds - the checker verdict stands",
                }))
                .unwrap(),
            )),
        )?;
        Ok(StopGreen::Banked(self.bank_pass(mission, answer_path, steps, *model_calls)?))
    }

    /// Mission banking on a green verdict: GoalUpdate + checkpoint + the
    /// passed result. One tail for every path that ends a green mission.
    fn bank_pass(
        &mut self,
        mission: &str,
        answer_path: &Path,
        steps: u32,
        model_calls: u32,
    ) -> Result<MissionResult, LoopError> {
        self.writer.append(
            EventBuilder::new(EventKind::GoalUpdate).payload(Payload::Inline(
                serde_json::to_vec(&serde_json::json!({"mission": mission, "done": true}))
                    .unwrap(),
            )),
        )?;
        self.checkpoint(steps, model_calls);
        Ok(MissionResult {
            passed: true,
            steps,
            model_calls,
            stream_id: self.stream_id,
            answer_path: answer_path.to_path_buf(),
            budget_killed: false,
            harness_error: None,
        })
    }

    /// Item 3 (synchronous legacy path, byte-compatible event shapes):
    /// checker-green -> verifier veto, the agent BLOCKS for the call.
    /// Kept as the default until the async seam clears its promotion gate,
    /// and as the degradation path when snapshot infrastructure fails.
    fn sync_verifier_round(
        &mut self,
        mission: &str,
        answer_path: &Path,
        round: u32,
        steps: u32,
        model_calls: &mut u32,
        pending_feedback: &mut Vec<String>,
    ) -> Result<StopGreen, LoopError> {
        let answer_text = std::fs::read_to_string(answer_path).unwrap_or_default();
        let vprompt = verifier::build_verifier_prompt(mission, &answer_text, &self.ledger, &self.prior_gaps);
        let verdict_tools = serde_json::json!([crate::toolschema::verdict_tool()]);
        match self.kernel.call_model_with("operator", None, &vprompt, Some(&verdict_tools)) {
            Ok(vout) => {
                *model_calls += 1;
                self.cost_total_micros += vout.cost_usd_micros.max(0) as u64;
                self.writer.append(
                    EventBuilder::new(EventKind::ModelCall)
                        .payload(Payload::Inline(
                            serde_json::to_vec(&serde_json::json!({
                                "role": "verifier", "round": round,
                                "prompt": vprompt, "tools": verdict_tools,
                                "completion": vout.completion,
                                "input_tokens": vout.input_tokens,
                                "output_tokens": vout.output_tokens,
                                "reasoning_tokens": vout.reasoning_tokens,
                                "reasoning_content": vout.reasoning_content,
                                "cached_tokens": vout.cached_tokens,
                                "cost_usd_micros": vout.cost_usd_micros,
                                "latency_ms": vout.latency_ms,
                            }))
                            .unwrap(),
                        ))
                        .latency_ms(vout.latency_ms)
                        .cost_usd_micros(vout.cost_usd_micros),
                )?;
                match verifier::parse_verdict(&vout.completion) {
                    Ok(v) => {
                        if v.refuted {
                            self.prior_gaps = v.findings.clone();
                            self.writer.append(
                                EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                                    serde_json::to_vec(&serde_json::json!({
                                        "why": "verifier", "round": round, "verdict": "refuted",
                                        "findings": v.findings, "blocking": v.blocking,
                                    }))
                                    .unwrap(),
                                )),
                            )?;
                            pending_feedback.push(format!(
                                "VERIFIER REFUTED (blocking={}): {}",
                                v.blocking,
                                v.findings.join("; ")
                            ));
                            return Ok(StopGreen::Continue);
                        }
                        self.prior_gaps.clear();
                        self.writer.append(
                            EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                                serde_json::to_vec(&serde_json::json!({
                                    "why": "verifier", "round": round, "verdict": "not_refuted",
                                }))
                                .unwrap(),
                            )),
                        )?;
                    }
                    Err(detail) => {
                        self.writer.append(
                            EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                                serde_json::to_vec(&serde_json::json!({
                                    "why": "verifier_error", "round": round,
                                    "detail": detail,
                                }))
                                .unwrap(),
                            )),
                        )?;
                    }
                }
            }
            Err(e) => {
                self.writer.append(
                    EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                        serde_json::to_vec(&serde_json::json!({
                            "why": "verifier_error", "round": round,
                            "detail": format!("verifier call failed: {e}"),
                        }))
                        .unwrap(),
                    )),
                )?;
            }
        }
        Ok(StopGreen::Banked(self.bank_pass(mission, answer_path, steps, *model_calls)?))
    }

    /// Gate-8 async round: snapshot the ws, serve a PROVABLY identical
    /// resubmission from the verdict cache, otherwise fire the audit on a
    /// worker thread and let the agent keep working. Snapshot
    /// infrastructure failure NEVER skips a needed audit - the round
    /// degrades to the synchronous path.
    fn async_verifier_round(
        &mut self,
        mission: &str,
        answer_path: &Path,
        round: u32,
        steps: u32,
        model_calls: &mut u32,
        pending_feedback: &mut Vec<String>,
    ) -> Result<StopGreen, LoopError> {
        let answer_text = std::fs::read_to_string(answer_path).unwrap_or_default();
        let ws = self.ws_path.clone().expect("async_verify sets ws_path");
        let snapshot_id = match self.world.as_ref().map(|w| w.snapshot(&ws)) {
            Some(Ok(rep)) => Some(rep.snapshot_id),
            Some(Err(e)) => {
                self.writer.append(
                    EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                        serde_json::to_vec(&serde_json::json!({
                            "why": "verifier_snapshot_error", "round": round,
                            "detail": format!("snapshot failed, verifying synchronously: {e:?}"),
                        }))
                        .unwrap(),
                    )),
                )?;
                return self.sync_verifier_round(
                    mission,
                    answer_path,
                    round,
                    steps,
                    model_calls,
                    pending_feedback,
                );
            }
            None => None,
        };
        let key = verifier::verdict_key(
            mission,
            &answer_text,
            &self.ledger.evidence_key(),
            &self.prior_gaps,
            snapshot_id.as_deref().unwrap_or(""),
        );
        if let Some(rec) = self.verdict_cache.get(&key).cloned() {
            // Identical artifact + answer + evidence + gaps: the recorded
            // verdict IS this audit's verdict - re-asking would resample
            // an already-drawn verdict, not add scrutiny.
            self.writer.append(
                EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                    serde_json::to_vec(&serde_json::json!({
                        "why": "verifier", "round": round, "verifier_cache_hit": true,
                        "verdict": if rec.refuted { "refuted" } else { "not_refuted" },
                    }))
                    .unwrap(),
                )),
            )?;
            return self.apply_recorded_verdict(
                rec,
                round,
                true,
                mission,
                answer_path,
                steps,
                model_calls,
                pending_feedback,
            );
        }
        let vprompt = verifier::build_verifier_prompt(mission, &answer_text, &self.ledger, &self.prior_gaps);
        let verdict_tools = serde_json::json!([crate::toolschema::verdict_tool()]);
        let Some(entry) = self.kernel.list_models().into_iter().find(|e| e.default) else {
            // no default model entry: degrade to the synchronous path
            return self.sync_verifier_round(
                mission,
                answer_path,
                round,
                steps,
                model_calls,
                pending_feedback,
            );
        };
        let rx = verifier::spawn_round(entry, vprompt, verdict_tools, round);
        self.pending_verdict = Some(verifier::PendingVerdict {
            round,
            rx,
            snapshot_id,
            key,
            fired: std::time::Instant::now(),
        });
        pending_feedback.push(format!(
            "checker green - your submission passed the mechanical gate and is now under adversarial audit (round {round}/{}). The workspace is snapshotted. Keep working: probe edge cases, run more checks, harden the patch. If the audit vetoes, the workspace returns to the audited snapshot (your work since then stays in your transcript).",
            verifier::VERIFIER_MAX_ROUNDS
        ));
        Ok(StopGreen::Continue)
    }

    /// Drain (blocking=false) or await (blocking=true) the pending async
    /// verdict. A DECIDED verdict always restores the audited snapshot
    /// first - the ws the agent repairs from (veto) and the ws the patch
    /// is extracted from (bank) are byte-identical to what the verifier
    /// judged.
    fn process_verdict(
        &mut self,
        blocking: bool,
        mission: &str,
        answer_path: &Path,
        steps: u32,
        model_calls: &mut u32,
        pending_feedback: &mut Vec<String>,
    ) -> Result<Option<MissionResult>, LoopError> {
        let msg = {
            let Some(p) = &self.pending_verdict else {
                return Ok(None);
            };
            if blocking {
                match p.rx.recv() {
                    Ok(m) => m,
                    Err(_) => verifier::VerdictMsg::CallFailed {
                        round: p.round,
                        detail: "verifier worker died without a verdict".to_string(),
                        prompt: String::new(),
                        tools: serde_json::Value::Null,
                    },
                }
            } else {
                match p.rx.try_recv() {
                    Ok(m) => m,
                    Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(None),
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        // a dead worker must not stall the mission:
                        // malfunction never blocks (same as a call failure)
                        verifier::VerdictMsg::CallFailed {
                            round: p.round,
                            detail: "verifier worker died without a verdict".to_string(),
                            prompt: String::new(),
                            tools: serde_json::Value::Null,
                        }
                    }
                }
            }
        };
        let p = self.pending_verdict.take().unwrap();
        match msg {
            verifier::VerdictMsg::Decided { round, v, out, prompt, tools } => {
                *model_calls += 1;
                self.cost_total_micros += out.cost_usd_micros.max(0) as u64;
                self.writer.append(
                    EventBuilder::new(EventKind::ModelCall)
                        .payload(Payload::Inline(
                            serde_json::to_vec(&serde_json::json!({
                                "role": "verifier", "round": round, "async": true,
                                "prompt": prompt, "tools": tools,
                                "completion": out.completion,
                                "input_tokens": out.input_tokens,
                                "output_tokens": out.output_tokens,
                                "reasoning_tokens": out.reasoning_tokens,
                                "reasoning_content": out.reasoning_content,
                                "cached_tokens": out.cached_tokens,
                                "cost_usd_micros": out.cost_usd_micros,
                                "latency_ms": out.latency_ms,
                            }))
                            .unwrap(),
                        ))
                        .latency_ms(out.latency_ms)
                        .cost_usd_micros(out.cost_usd_micros),
                )?;
                self.verdict_cache.insert(p.key.clone(), v.clone());
                self.restore_audited(&p, round)?;
                match self.apply_recorded_verdict(
                    v,
                    round,
                    false,
                    mission,
                    answer_path,
                    steps,
                    model_calls,
                    pending_feedback,
                )? {
                    StopGreen::Continue => Ok(None),
                    StopGreen::Banked(r) => Ok(Some(r)),
                }
            }
            verifier::VerdictMsg::Malformed { round, detail, out, prompt, tools } => {
                if let Some(out) = out {
                    *model_calls += 1;
                    self.cost_total_micros += out.cost_usd_micros.max(0) as u64;
                    self.writer.append(
                        EventBuilder::new(EventKind::ModelCall)
                            .payload(Payload::Inline(
                                serde_json::to_vec(&serde_json::json!({
                                    "role": "verifier", "round": round, "async": true,
                                    "prompt": prompt, "tools": tools,
                                    "completion": out.completion,
                                    "input_tokens": out.input_tokens,
                                    "output_tokens": out.output_tokens,
                                    "reasoning_tokens": out.reasoning_tokens,
                                    "reasoning_content": out.reasoning_content,
                                    "cached_tokens": out.cached_tokens,
                                    "cost_usd_micros": out.cost_usd_micros,
                                    "latency_ms": out.latency_ms,
                                }))
                                .unwrap(),
                            ))
                            .latency_ms(out.latency_ms)
                            .cost_usd_micros(out.cost_usd_micros),
                    )?;
                }
                self.writer.append(
                    EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                        serde_json::to_vec(&serde_json::json!({
                            "why": "verifier_error", "round": round,
                            "detail": detail,
                        }))
                        .unwrap(),
                    )),
                )?;
                // malfunction never blocks: the checker verdict stands
                Ok(Some(self.bank_pass(mission, answer_path, steps, *model_calls)?))
            }
            verifier::VerdictMsg::CallFailed { round, detail, .. } => {
                self.writer.append(
                    EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                        serde_json::to_vec(&serde_json::json!({
                            "why": "verifier_error", "round": round,
                            "detail": detail,
                        }))
                        .unwrap(),
                    )),
                )?;
                Ok(Some(self.bank_pass(mission, answer_path, steps, *model_calls)?))
            }
        }
    }

    /// Restore the ws to the audited snapshot (both verdict outcomes).
    /// Restore is hash-verified; a failure is a HARNESS failure (the patch
    /// can no longer be tied to the audit) and aborts the mission as such.
    fn restore_audited(&mut self, p: &verifier::PendingVerdict, round: u32) -> Result<(), LoopError> {
        let (Some(w), Some(ws), Some(snap)) = (
            self.world.as_ref(),
            self.ws_path.as_ref(),
            p.snapshot_id.as_deref(),
        ) else {
            return Ok(());
        };
        match w.restore_replace(snap, ws) {
            Ok(rep) => {
                self.writer.append(
                    EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                        serde_json::to_vec(&serde_json::json!({
                            "why": "verifier_restore", "round": round,
                            "snapshot": snap, "took_ms": rep.took_ms,
                        }))
                        .unwrap(),
                    )),
                )?;
                Ok(())
            }
            Err(e) => {
                let msg = format!("verifier snapshot restore failed (round {round}, snapshot {snap}): {e:?}");
                self.writer.append(
                    EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                        serde_json::to_vec(&serde_json::json!({
                            "why": "verifier_restore_error", "round": round,
                            "detail": msg,
                        }))
                        .unwrap(),
                    )),
                )?;
                Err(LoopError::ModelOutput(msg))
            }
        }
    }

    /// Apply a decided verdict (async round or cache replay). Event shapes
    /// match the synchronous path plus the `cached` marker.
    fn apply_recorded_verdict(
        &mut self,
        v: verifier::RecordedVerdict,
        round: u32,
        cached: bool,
        mission: &str,
        answer_path: &Path,
        steps: u32,
        model_calls: &mut u32,
        pending_feedback: &mut Vec<String>,
    ) -> Result<StopGreen, LoopError> {
        if v.refuted {
            self.prior_gaps = v.findings.clone();
            self.writer.append(
                EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                    serde_json::to_vec(&serde_json::json!({
                        "why": "verifier", "round": round, "verdict": "refuted",
                        "findings": v.findings, "blocking": v.blocking, "cached": cached,
                    }))
                    .unwrap(),
                )),
            )?;
            pending_feedback.push(format!(
                "VERIFIER REFUTED (blocking={}): {}. The workspace was restored to the audited snapshot; your work since the submit is in your transcript.",
                v.blocking,
                v.findings.join("; ")
            ));
            return Ok(StopGreen::Continue);
        }
        self.prior_gaps.clear();
        self.writer.append(
            EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                serde_json::to_vec(&serde_json::json!({
                    "why": "verifier", "round": round, "verdict": "not_refuted", "cached": cached,
                }))
                .unwrap(),
            )),
        )?;
        Ok(StopGreen::Banked(self.bank_pass(mission, answer_path, steps, *model_calls)?))
    }
}

/// The stop_green tail's two outcomes.
enum StopGreen {
    /// refuted / audit fired - the agent keeps working
    Continue,
    /// mission ends green
    Banked(MissionResult),
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
