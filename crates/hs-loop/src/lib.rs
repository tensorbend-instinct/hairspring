//! HAIRSPRING gate 3 - the inner loop with semantic feedback (spec 6).
//!
//! One step: observe -> drain_feedback -> assemble -> model.call ->
//! validate -> submit -> checker verdict. The verdict is recorded as a
//! feedback event in BOTH ablation arms; in the ON arm it is also injected
//! into the next step's context (recorded as context_inject: what entered
//! the window, and why). Feedback never costs a model round trip.

pub mod realmodel;
pub mod mcpbridge;
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
            let mut volatile = format!(
                "ATTEMPT: {step}\nANSWER_PATH: {}\nARTIFACT: {}\n",
                answer_path.display(),
                if artifact.is_empty() {
                    "<none>"
                } else {
                    artifact.trim()
                }
            );
            let mut injected = false;
            if self.feedback_injection && !drained.is_empty() {
                volatile.push_str("FEEDBACK:\n");
                for f in &drained {
                    volatile.push_str(&format!("- {f}\n"));
                }
                injected = true;
            }
            // Mission memory per spec v4: "Memory, recovery, evaluation...
            // are all read paths over the same log." The transcript is READ
            // BACK from this stream's own event record (ToolCall payloads
            // carry args+result), newest-first capped to 60KB - no parallel
            // store, and it survives process restarts/freeze recovery.
            // Part of the feedback channel: no injection, no memory.
            if self.feedback_injection {
                if let Ok(reader) = hs_log::StreamReader::open(&self.log_root, self.stream_id) {
                    if let Ok(events) = reader.events() {
                        // Collect ALL transcript lines newest-first, tagged
                        // with their source event for audit refs.
                        const WINDOW_CAP: usize = 60_000;
                        const HIGH_WATER: usize = WINDOW_CAP * 85 / 100;
                        const TAIL_BUDGET: usize = WINDOW_CAP * 60 / 100;
                        let mut lines: Vec<(u64, uuid::Uuid, String)> = vec![];
                        for e in events.iter().rev() {
                            if e.kind != hs_core::EventKind::ToolCall {
                                continue;
                            }
                            // resolve Inline AND BlobRef payloads: hs-log
                            // promotes large results to blob refs on append,
                            // and skipping them dropped big tool outputs
                            // from mission memory entirely
                            if let Ok(bytes) = reader.resolve_payload(e) {
                                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes)
                                {
                                    let mut line = format!(
                                        "{}({}) => {}",
                                        v["plugin"].as_str().unwrap_or("?"),
                                        v["args"],
                                        v["result"]
                                    );
                                    if line.len() > 20_000 {
                                        line.truncate(20_000);
                                        line.push_str("...[truncated]");
                                    }
                                    lines.push((e.seq, e.event_id, line));
                                }
                            }
                        }
                        let total: usize = lines.iter().map(|(_, _, l)| l.len() + 8).sum();
                        let mut entries: Vec<String> = vec![];
                        if total > HIGH_WATER {
                            // GATE 9f (spec v5): compact on window PRESSURE,
                            // not on a fixed schedule. The recent tail stays
                            // verbatim; older segments distill into a summary
                            // that links back to the source event range, so
                            // distillation never destroys auditability.
                            let mut budget = TAIL_BUDGET;
                            let mut kept: Vec<String> = vec![];
                            let mut compacted: Vec<(u64, uuid::Uuid, String)> = vec![];
                            for (seq, id, line) in lines {
                                if compacted.is_empty() && line.len() + 8 <= budget {
                                    budget -= line.len() + 8;
                                    kept.push(line);
                                } else {
                                    compacted.push((seq, id, line));
                                }
                            }
                            if !compacted.is_empty() {
                                let lo = compacted.last().unwrap();
                                let hi = compacted.first().unwrap();
                                let mut counts: std::collections::BTreeMap<String, usize> =
                                    Default::default();
                                for (_, _, l) in &compacted {
                                    let plugin =
                                        l.split('(').next().unwrap_or("?").to_string();
                                    *counts.entry(plugin).or_insert(0) += 1;
                                }
                                let tally = counts
                                    .iter()
                                    .map(|(p, n)| format!("{p}x{n}"))
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                let summary = format!(
                                    "COMPACTED {} earlier tool calls (events seq {}..{}, refs {}..{}): {}",
                                    compacted.len(),
                                    lo.0,
                                    hi.0,
                                    lo.1,
                                    hi.1,
                                    tally
                                );
                                // record(context_inject, why=pressure)
                                let body = format!(
                                    "context_inject why=pressure compacted={} range=seq{}..seq{}",
                                    compacted.len(),
                                    lo.0,
                                    hi.0
                                );
                                let _ = self.writer.append(
                                    EventBuilder::new(EventKind::ContextInject)
                                        .payload(Payload::Inline(body.into_bytes())),
                                );
                                kept.reverse();
                                entries.push(summary);
                                entries.extend(kept);
                            } else {
                                kept.reverse();
                                entries = kept;
                            }
                        } else {
                            // under the watermark: everything verbatim,
                            // newest-first fill (never breaks when it fits)
                            let mut budget = WINDOW_CAP;
                            for (_, _, line) in lines {
                                if line.len() + 8 > budget {
                                    break;
                                }
                                budget -= line.len() + 8;
                                entries.push(line);
                            }
                            entries.reverse();
                        }
                        if !entries.is_empty() {
                            ctx.push_str("TRANSCRIPT (earlier tool calls):\n");
                            for e in &entries {
                                ctx.push_str(&format!("- {e}\n"));
                            }
                        }
                    }
                }
            }

            ctx.push_str(&volatile);

            // the only model round trip in the step
            let out = self.kernel.call_model("operator", None, &ctx)?;
            model_calls += 1;
            self.cost_total_micros += out.cost_usd_micros.max(0) as u64;
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
                    return Ok(MissionResult {
                        passed: false,
                        steps,
                        model_calls,
                        stream_id: self.stream_id,
                        answer_path,
                        budget_killed: true,
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
                Some((tool, args)) => match self.kernel.call_tool("operator", &tool, args.clone()) {
                    Ok(tool_out) => {
                        self.writer.append(
                            EventBuilder::new(EventKind::ToolCall)
                                .payload(Payload::Inline(
                                    serde_json::to_vec(&serde_json::json!({
                                        "plugin": tool, "args": args, "result": tool_out.output,
                                    }))
                                    .unwrap(),
                                ))
                                .latency_ms(tool_out.latency_ms),
                        )?;
                        if tool == "answer.write" {
                            wrote_answer = true;
                        }
                        // non-write results reach future steps via the
                        // log-sourced TRANSCRIPT above (read back from this
                        // stream's own ToolCall events), not a side channel
                        None
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
            self.writer.append(
                EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                    serde_json::to_vec(&serde_json::json!({
                        "checker": "checker.run", "task_id": mission,
                        "passed": passed, "error": error,
                    }))
                    .unwrap(),
                )),
            )?;
            if passed {
                self.writer.append(
                    EventBuilder::new(EventKind::GoalUpdate).payload(Payload::Inline(
                        serde_json::to_vec(&serde_json::json!({"mission": mission, "done": true}))
                            .unwrap(),
                    )),
                )?;
                return Ok(MissionResult {
                    passed: true,
                    steps,
                    model_calls,
                    stream_id: self.stream_id,
                    answer_path,
                    budget_killed: false,
                });
            }
            pending_feedback.push(error);
        }
        Ok(MissionResult {
            passed: false,
            steps,
            model_calls,
            stream_id: self.stream_id,
            answer_path,
            budget_killed: false,
        })
    }
}
