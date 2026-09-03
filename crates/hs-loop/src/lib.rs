//! HAIRSPRING gate 3 - the inner loop with semantic feedback (spec 6).
//!
//! One step: observe -> drain_feedback -> assemble -> model.call ->
//! validate -> submit -> checker verdict. The verdict is recorded as a
//! feedback event in BOTH ablation arms; in the ON arm it is also injected
//! into the next step's context (recorded as context_inject: what entered
//! the window, and why). Feedback never costs a model round trip.

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
}

pub struct InnerLoop {
    kernel: Kernel,
    writer: StreamWriter,
    stream_id: uuid::Uuid,
    log_root: PathBuf,
    feedback_injection: bool,
    max_steps: u32,
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
        })
    }

    pub fn stream_id(&self) -> uuid::Uuid {
        self.stream_id
    }

    /// Run one mission to a checker verdict or the step cap.
    pub fn run_mission(&mut self, mission: &str) -> Result<MissionResult, LoopError> {
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

            // assemble
            let mut ctx = format!(
                "MISSION: {mission}\nATTEMPT: {step}\nANSWER_PATH: {}\nARTIFACT: {}\n",
                answer_path.display(),
                if artifact.is_empty() {
                    "<none>"
                } else {
                    artifact.trim()
                }
            );
            let mut injected = false;
            if self.feedback_injection && !drained.is_empty() {
                ctx.push_str("FEEDBACK:\n");
                for f in &drained {
                    ctx.push_str(&format!("- {f}\n"));
                }
                injected = true;
            }

            // the only model round trip in the step
            let out = self.kernel.call_model("operator", None, &ctx)?;
            model_calls += 1;
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

            // validate: the plan must be a single well-formed action
            let plan: serde_json::Value = serde_json::from_str(&out.completion)
                .map_err(|e| LoopError::ModelOutput(format!("unparsable completion: {e}")))?;
            let tool = plan["tool"]
                .as_str()
                .ok_or_else(|| LoopError::ModelOutput("no tool".into()))?;
            let args = plan["args"].clone();

            // submit
            self.kernel.call_tool("operator", tool, args)?;

            // the world answers (checker = ground truth at this gate)
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
        })
    }
}
