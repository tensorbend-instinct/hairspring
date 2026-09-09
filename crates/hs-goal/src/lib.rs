//! HAIRSPRING gate 4 - outer loop: Goal Mode, budgets, gateway (spec 5-6).
//!
//! Completion authority (openJiuwen Goal Mode):
//!   `SelfDeclared` - the model's say-so completes the mission.
//!   Independent  - only checkers complete the mission; say-so is noise.
//!   Hybrid       - say-so triggers an immediate checker verdict; the
//!                  checker decides.
//! Checker trigger (not every step): the artifact changed, or the model
//! declared done. Budgets read cost from the log's own records. Gateway
//! events arrive async via an append-only inbox file and are handled at
//! step boundaries; progress (log + artifact) is never destroyed.

use hs_core::{EventBuilder, EventKind, Payload};
use hs_kernel::{Kernel, KernelError};
use hs_log::{LogError, StreamWriter};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum GoalError {
    Kernel(KernelError),
    Log(LogError),
    ModelOutput(String),
    Io(std::io::Error),
}
impl From<KernelError> for GoalError {
    fn from(e: KernelError) -> Self {
        GoalError::Kernel(e)
    }
}
impl From<LogError> for GoalError {
    fn from(e: LogError) -> Self {
        GoalError::Log(e)
    }
}
impl From<std::io::Error> for GoalError {
    fn from(e: std::io::Error) -> Self {
        GoalError::Io(e)
    }
}
impl std::fmt::Display for GoalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Kernel(e) => write!(f, "kernel: {e}"),
            Self::Log(e) => write!(f, "log: {e}"),
            Self::ModelOutput(e) => write!(f, "model output: {e}"),
            Self::Io(e) => write!(f, "io: {e}"),
        }
    }
}
impl std::error::Error for GoalError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionMode {
    SelfDeclared,
    Independent,
    Hybrid,
}

#[derive(Debug, Clone)]
pub struct Budget {
    pub max_steps: u32,
    pub max_cost_usd_micros: i64,
}

#[derive(Debug, Clone)]
pub struct Goal {
    pub goal_id: uuid::Uuid,
    pub spec: String,
    pub completion_mode: CompletionMode,
    pub checkers: Vec<String>,
    pub budget: Budget,
}
impl Goal {
    #[must_use]
    pub fn new(spec: &str, completion_mode: CompletionMode, budget: Budget) -> Self {
        Goal {
            goal_id: uuid::Uuid::new_v4(),
            spec: spec.to_string(),
            completion_mode,
            checkers: vec!["goalchecker.run".into()],
            budget,
        }
    }
}

#[derive(Debug)]
pub enum MissionOutcome {
    Passed { steps: u32 },
    FailedHonest { steps: u32 },
    Cancelled { at_step: u32 },
    BudgetExceeded { at_step: u32 },
}

pub struct OuterLoop {
    kernel: Kernel,
    writer: StreamWriter,
    stream_id: uuid::Uuid,
    log_root: PathBuf,
    step_delay_ms: u64,
    completions_accepted_on_say_so: u32,
    false_completions_caught: u32,
}

impl OuterLoop {
    pub fn new(kernel: Kernel, log_root: &Path, step_delay_ms: u64) -> Result<Self, GoalError> {
        let stream_id = uuid::Uuid::new_v4();
        let writer = StreamWriter::create(log_root, stream_id)?;
        Ok(OuterLoop {
            kernel,
            writer,
            stream_id,
            log_root: log_root.to_path_buf(),
            step_delay_ms,
            completions_accepted_on_say_so: 0,
            false_completions_caught: 0,
        })
    }

    #[must_use]
    pub fn with_step_delay_ms(mut self, ms: u64) -> Self {
        self.step_delay_ms = ms;
        self
    }
    pub fn stream_id(&self) -> uuid::Uuid {
        self.stream_id
    }
    pub fn gateway_inbox(&self) -> PathBuf {
        self.log_root.join("gateway-inbox.jsonl")
    }
    pub fn completions_accepted_on_say_so(&self) -> u32 {
        self.completions_accepted_on_say_so
    }
    pub fn false_completions_caught(&self) -> u32 {
        self.false_completions_caught
    }

    fn record(&mut self, kind: EventKind, body: serde_json::Value) -> Result<(), GoalError> {
        self.record_ev(kind, body, 0, 0)
    }

    fn record_ev(
        &mut self,
        kind: EventKind,
        body: serde_json::Value,
        latency_ms: u32,
        cost: i64,
    ) -> Result<(), GoalError> {
        self.writer.append(
            EventBuilder::new(kind)
                .payload(Payload::Inline(
                    serde_json::to_vec(&body).expect("goal event bodies serialize"),
                ))
                .latency_ms(latency_ms)
                .cost_usd_micros(cost),
        )?;
        Ok(())
    }

    /// Drain gateway events arrived since the last step boundary.
    fn drain_gateway(&self) -> Vec<serde_json::Value> {
        let path = self.gateway_inbox();
        if !path.exists() {
            return vec![];
        }
        let read = std::fs::read_to_string(&path).unwrap_or_default();
        // truncate after read: each event handled exactly once
        let _ = std::fs::write(&path, b"");
        read.lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    pub fn run(&mut self, goal: &Goal) -> Result<MissionOutcome, GoalError> {
        let mut spec = goal.spec.clone();
        let mut cost_usd_micros: i64 = 0;
        self.record(EventKind::GoalUpdate, serde_json::json!({
            "goal_id": goal.goal_id, "spec": spec, "mode": format!("{:?}", goal.completion_mode),
            "budget": {"max_steps": goal.budget.max_steps, "max_cost_usd_micros": goal.budget.max_cost_usd_micros},
        }))?;

        let mut attempt: u32 = 0;
        let mut step: u32 = 0;
        while step < goal.budget.max_steps {
            step += 1;
            attempt += 1;
            // gateway events land at the step boundary, never mid-step
            for ev in self.drain_gateway() {
                match ev["type"].as_str() {
                    Some("cancel") => {
                        self.record(
                            EventKind::Message,
                            serde_json::json!({"gateway": "cancel", "at_step": step}),
                        )?;
                        return Ok(MissionOutcome::Cancelled { at_step: step });
                    }
                    Some("redirect") => {
                        let new_spec = ev["new_spec"].as_str().unwrap_or("").to_string();
                        self.record(
                            EventKind::Message,
                            serde_json::json!({"gateway": "redirect", "at_step": step}),
                        )?;
                        self.record(
                            EventKind::GoalUpdate,
                            serde_json::json!({
                                "goal_id": goal.goal_id, "spec": new_spec, "redirect_from": spec,
                            }),
                        )?;
                        spec = new_spec;
                        attempt = 0; // a retargeted mission starts its attempt sequence over
                    }
                    Some("add_task") => {
                        self.record(
                            EventKind::Message,
                            serde_json::json!({"gateway": "add_task", "at_step": step}),
                        )?;
                    }
                    _ => {}
                }
            }

            let answer_path = self.log_root.join("work").join(&spec).join("answer.txt");
            std::fs::create_dir_all(
                answer_path
                    .parent()
                    .expect("joined path always has a parent"),
            )?;
            let artifact_before = std::fs::read_to_string(&answer_path).unwrap_or_default();
            let ctx = format!(
                "SPEC: {spec}\nATTEMPT: {attempt}\nANSWER_PATH: {}\nARTIFACT: {}\n",
                answer_path.display(),
                if artifact_before.is_empty() {
                    "<none>"
                } else {
                    artifact_before.trim()
                }
            );

            let out = self.kernel.call_model("operator", None, &ctx)?;
            cost_usd_micros += out.cost_usd_micros;
            self.record_ev(
                EventKind::ModelCall,
                serde_json::json!({
                    "model": out.model, "prompt": ctx, "completion": out.completion,
                    "input_tokens": out.input_tokens, "output_tokens": out.output_tokens,
                }),
                out.latency_ms,
                out.cost_usd_micros,
            )?;
            let plan: serde_json::Value = serde_json::from_str(&out.completion)
                .map_err(|e| GoalError::ModelOutput(e.to_string()))?;

            let mut artifact_changed = false;
            let mut declared_done = plan["done"].as_bool().unwrap_or(false);
            if !declared_done {
                let tool = plan["tool"]
                    .as_str()
                    .ok_or_else(|| GoalError::ModelOutput("no tool".into()))?;
                let r = self
                    .kernel
                    .call_tool("operator", tool, plan["args"].clone())?;
                cost_usd_micros += r.output["cost_usd_micros"].as_i64().unwrap_or(0);
                self.record_ev(
                    EventKind::ToolCall,
                    serde_json::json!({
                        "plugin": tool, "args": plan["args"].clone(), "result": r.output,
                    }),
                    r.latency_ms,
                    0,
                )?;
                artifact_changed =
                    std::fs::read_to_string(&answer_path).unwrap_or_default() != artifact_before;
            }

            // Completion authority by mode. Checker triggers: artifact
            // changed, or done declared - not every step.
            let mut passed = false;
            match goal.completion_mode {
                CompletionMode::SelfDeclared => {
                    if declared_done {
                        self.completions_accepted_on_say_so += 1;
                        passed = true;
                    }
                }
                CompletionMode::Independent => {
                    if artifact_changed {
                        passed = self.check(goal, &spec, &answer_path)?;
                    }
                    // say-so is ignored entirely
                    let _ = declared_done;
                    declared_done = false;
                }
                CompletionMode::Hybrid => {
                    if declared_done {
                        let verdict = self.check(goal, &spec, &answer_path)?;
                        if verdict {
                            passed = true;
                        } else {
                            self.false_completions_caught += 1;
                            self.record(
                                EventKind::Observation,
                                serde_json::json!({
                                    "false_completion_caught": true, "spec": spec, "at_step": step,
                                }),
                            )?;
                        }
                    } else if artifact_changed {
                        passed = self.check(goal, &spec, &answer_path)?;
                    }
                }
            }
            if passed {
                self.record(
                    EventKind::GoalUpdate,
                    serde_json::json!({
                        "goal_id": goal.goal_id, "spec": spec, "done": true, "at_step": step,
                    }),
                )?;
                return Ok(MissionOutcome::Passed { steps: step });
            }

            // budget gate: checkpoint, then hand to outer
            if cost_usd_micros > goal.budget.max_cost_usd_micros {
                self.record(EventKind::BudgetUpdate, serde_json::json!({
                    "goal_id": goal.goal_id, "cost_usd_micros": cost_usd_micros, "exceeded": true,
                }))?;
                self.record(
                    EventKind::Decision,
                    serde_json::json!({
                        "breakpoint": true, "reason": "budget exceeded", "at_step": step,
                    }),
                )?;
                return Ok(MissionOutcome::BudgetExceeded { at_step: step });
            }

            if self.step_delay_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(self.step_delay_ms));
            }
        }
        Ok(MissionOutcome::FailedHonest {
            steps: goal.budget.max_steps,
        })
    }

    fn check(&mut self, goal: &Goal, spec: &str, answer_path: &Path) -> Result<bool, GoalError> {
        for checker in &goal.checkers {
            let v = self.kernel.call_tool(
                "operator",
                checker,
                serde_json::json!({"spec": spec, "path": answer_path}),
            )?;
            let ok = v.output["passed"].as_bool().unwrap_or(false);
            self.record(
                EventKind::Feedback,
                serde_json::json!({
                    "checker": checker, "spec": spec, "passed": ok,
                    "error": v.output["error"].as_str().unwrap_or(""),
                }),
            )?;
            if !ok {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
