//! HAIRSPRING gate 3 - the inner loop with semantic feedback (spec 6).
//!
//! One step: observe -> `drain_feedback` -> assemble -> model.call ->
//! validate -> submit -> checker verdict. The verdict is recorded as a
//! feedback event in BOTH ablation arms; in the ON arm it is also injected
//! into the next step's context (recorded as `context_inject`: what entered
//! the window, and why). Feedback never costs a model round trip.

pub mod assembler;
pub mod critic;
pub mod editapply;
pub mod evolve;
pub mod goal;
pub mod ledger;
pub mod mcpbridge;
pub mod mission_time;
pub mod publication;
pub mod projectroot;
pub mod msgfmt;
pub mod realmodel;
pub mod repexec;
pub mod repl;
pub mod repotools;
pub mod selfcheck;
pub mod termexec;
pub mod sweprompt;
pub mod toolschema;
pub mod setup;
pub mod tui;
pub mod tui_views;
pub mod uipaint;

use hs_core::{EventBuilder, EventKind, Payload};
use hs_kernel::{Kernel, KernelError, ToolCallOutcome};
use hs_log::{LogError, StreamWriter};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum LoopError {
    Kernel(KernelError),
    Log(LogError),
    ModelOutput(String),
    Io(std::io::Error),
    /// `require_visibility` refused the run: kernel has no log root (the
    /// production startup gate - no blind runs).
    Visibility(String),
    /// The world plane (snapshot/restore) refused the operation.
    World(String),
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
            Self::Visibility(e) => write!(f, "visibility: {e}"),
            Self::World(e) => write!(f, "world: {e}"),
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
    /// M21: this mission's own provider-reported spend (micro-USD) -
    /// the delta of the loop's cumulative counter over the mission.
    /// The TUI done line prints THIS; the session total lives on
    /// `total_cost_micros()`.
    pub cost_micros: u64,
    /// D5: this mission's conservative list-rate spend (no cache
    /// credit) - the figure budget guards bind. Both are booked.
    pub conservative_cost_micros: u64,
    /// Some(msg) when the mission aborted on a harness failure (phase 1:
    /// a plugin declared `PluginDead` by the supervisor). The message names
    /// the plugin and the real cause. Harness-aborted missions book their
    /// steps-so-far; they are infrastructure failures, not model failures.
    pub harness_error: Option<String>,
    /// How the mission resolved (feedback integrity F2/F3): "verified"
    /// (checker green + verifier audited and accepted), "`ratchet_capped`"
    /// (checker green but the verifier refuted every round and the cap
    /// freed the submit), "`verifier_malfunction`" (checker green, the
    /// audit itself errored and never blocked), "`budget_killed`",
    /// "`steps_exhausted`", "`harness_error`". A capped or malfunction pass
    /// is never byte-identical to an audited one again.
    pub outcome: String,
}

/// A delegated child still running (async delegation, Eric ruling
/// 2026-09-08): the loop booked its Spawn at mint time and learns the
/// outcome by polling `agent.spawn_poll` at step boundaries.
pub struct PendingChild {
    pub child: uuid::Uuid,
    pub mission: String,
    pub model: String,
}

/// B1 (v5 D3): one in-flight speculation - the args the prefetch
/// predictor expects the next `memory.recall` to carry, its already
/// computed result, and the estimated token cost the fetch burned.
/// Resolved to a booked `Prefetch` event (hit/miss + cost) at the next
/// recall, or to a not-consumed miss at mission close.
struct PrefetchCache {
    args: serde_json::Value,
    result: serde_json::Value,
    tokens_est: u32,
}

/// B1 (v5 D3): minimum resolved prefetches before self-retirement is
/// evaluated, and the hit-rate crossover below which the predictor
/// retires itself ("if the K hit rate falls below the logging cost
/// crossover, the prefetch predictor turns itself off - that is the
/// system working"). Tunable; booked on every Prefetch event.
const PREFETCH_MIN_SAMPLES: u32 = 4;
const PREFETCH_COST_CROSSOVER: f64 = 0.5;

pub struct InnerLoop {
    kernel: Kernel,
    writer: StreamWriter,
    stream_id: uuid::Uuid,
    log_root: PathBuf,
    feedback_injection: bool,
    max_steps: u32,
    cost_total_micros: u64,
    /// D5: cumulative CONSERVATIVE list-rate cost (no cache credit).
    /// The budget guard binds this counter; the provider-reported
    /// counter above is kept alongside for honest reporting.
    conservative_cost_total_micros: u64,
    /// M21: cumulative cost at the current mission's start.
    mission_cost_start: u64,
    /// D5: conservative counterpart of `mission_cost_start`.
    mission_conservative_start: u64,
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
    /// the last refuted round handed back (the next round's `PRIOR_GAPS`)
    verifier_rounds: u32,
    /// Spec 5.2 goal record: how completion was arbitrated on the close
    /// that ends this mission ("hybrid" = say-so triggered the checkers,
    /// the checkers decided). None until a checker-driven close runs.
    completion_mode: Option<&'static str>,
    /// DISC judgment accounting for the mission in flight (reset at
    /// mission start; booked on the close).
    idi_interventions: u32,
    idi_detections: u32,
    idi_repairs: u32,
    prior_gaps: Vec<String>,
    /// Fix 4: mission wall budget (secs) + start instant, for the per-step
    /// "T-minus" header. None = wall not tracked (old behavior).
    wall_secs: Option<u64>,
    /// B3 (v5 2.5): gateway task inbox - goals injected mid-run are
    /// drained at step boundaries, booked as Message (gateway traffic),
    /// and queued to run after the current mission closes.
    task_inbox: Option<PathBuf>,
    /// Goals drained from the task inbox, in injection order.
    queued_tasks: std::collections::VecDeque<String>,
    /// Gap #2: operator steering inbox - a file of lines the loop drains
    /// at every step boundary into the volatile tail (mid-mission user
    /// turns). None = no operator channel (old behavior).
    steering_inbox: Option<PathBuf>,
    /// Gap #2: operator interrupt flag - when this file exists at a step
    /// boundary the mission stops cleanly with outcome "interrupted".
    interrupt_file: Option<PathBuf>,
    mission_started: Option<std::time::Instant>,
    /// Native tool schemas delivered to the provider's tools parameter on
    /// the operator call (native tool calling; Eric 2026-09-05). None = the
    /// model gets no tools param (legacy/text missions, unit fixtures).
    tools: Option<serde_json::Value>,
    /// Eric's five #4: operator model override (None = config
    /// default). Applies to every operator-subject call: mission
    /// steps, the verdict audit, and distillation.
    model_override: Option<String>,
    /// UI batch 1: typed mission UI events for the REPL painter.
    /// None = silent (old behavior).
    ui_sink: Option<uipaint::UiSink>,
    /// M10: last model name seen, so `ModelCallStart` can carry it before
    /// the call returns.
    last_model: Option<String>,
    /// B1 (v5 D3): prefetch predictor state - the outstanding
    /// speculation, its resolved hit/miss tallies, and whether the
    /// predictor is still running (it retires itself below
    /// `PREFETCH_COST_CROSSOVER` once `PREFETCH_MIN_SAMPLES` resolve).
    prefetch: Option<PrefetchCache>,
    prefetch_hits: u32,
    prefetch_misses: u32,
    prefetch_enabled: bool,
    /// 7.5: retirement knobs - session-overridable via
    /// `set_prefetch_knobs` (the promoted policy overlays them); the
    /// compiled-in constants are the defaults, not the law.
    prefetch_min_samples: u32,
    prefetch_cost_crossover: f64,
    /// B1: close-time distillation floor - the seq of the LAST mission
    /// close (seeded at stream end for resumed/forked streams). Extraction
    /// slices above it, so each mission's K record carries only its own
    /// edits and provenance on the shared session stream.
    distill_floor: u64,
    /// B2 (v5 gate 6): the shared world plane, one world stream inside
    /// this session's substrate log root. Attached via `attach_world`
    /// (REPL sessions always); world.* tools dispatch natively below.
    world: Option<hs_world::World>,
    /// Async delegation: children spawned by THIS mission, still
    /// running. Joined before a passing close; their costs land in
    /// this mission's books.
    pending_children: Vec<PendingChild>,
    /// This loop's delegation depth: seeded from `HS_SWARM_DEPTH` for a
    /// root session, set explicitly for children (their loops share
    /// the parent's plugin process - env is process-global and would
    /// race sibling threads). Injected into agent.spawn args; the
    /// model never fabricates it.
    swarm_depth: u32,
}

/// D1/W2: input budget from the VERIFIED provider context, minus an
/// output/reasoning reserve. kimi-k3: 1M-token context per Moonshot's
/// platform docs (models-overview, verified 2026-09-05); 64k reserve for
/// K3's always-on reasoning + output. Unknown models fall back
/// conservatively; --context-budget-tokens overrides.
pub const DEFAULT_CONTEXT_BUDGET_TOKENS: usize = 983_040;

#[must_use]
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
            conservative_cost_total_micros: 0,
            mission_cost_start: 0,
            mission_conservative_start: 0,
            budget_micros: None,
            tools: None,
            model_override: None,
            pending_children: Vec::new(),
            swarm_depth: std::env::var("HS_SWARM_DEPTH")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            progress_path: None,
            ledger: Default::default(),
            context_budget_chars: DEFAULT_CONTEXT_BUDGET_TOKENS * 4,
            memory_store: None,
            goal: None,
            dead_tools: Default::default(),
            doom_nudges: Default::default(),
            guardrail_escalator: Default::default(),
            verifier_rounds: 0,
            completion_mode: None,
            idi_interventions: 0,
            idi_detections: 0,
            idi_repairs: 0,
            prior_gaps: vec![],
            wall_secs: None,
            steering_inbox: None,
            task_inbox: None,
            queued_tasks: std::collections::VecDeque::new(),
            interrupt_file: None,
            mission_started: None,
            ui_sink: None,
            last_model: None,
            prefetch: None,
            prefetch_hits: 0,
            prefetch_misses: 0,
            prefetch_enabled: true,
            prefetch_min_samples: PREFETCH_MIN_SAMPLES,
            prefetch_cost_crossover: PREFETCH_COST_CROSSOVER,
            distill_floor: 0,
            world: None,
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
        // B1: a resumed/forked stream already holds the parent's mission
        // events; without this floor the first close here would re-distill
        // them into THIS mission's record (same overlap defect class).
        //
        // Accounting reset fix (Eric 2026-09-10): the same pass folds the
        // stream's priced ModelCall events back into the cumulative cost
        // counters, so a resumed session reports (and budgets against)
        // what the stream actually spent before the restart - pre-fix
        // both counters read 0 after :resume and the budget guard reset
        // with the display.
        let mut distill_floor = 0;
        let mut restored_cost = 0u64;
        let mut restored_conservative = 0u64;
        if let Ok(r) = hs_log::StreamReader::open(log_root, stream_id) {
            if let Ok(ev) = r.events() {
                distill_floor = ev.last().map(|e| e.seq).unwrap_or(0);
                for e in &ev {
                    if e.kind != hs_core::EventKind::ModelCall {
                        continue;
                    }
                    let Ok(b) = r.resolve_payload(e) else { continue };
                    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) else {
                        continue;
                    };
                    let micros = |k: &str| {
                        v.get(k)
                            .and_then(serde_json::Value::as_i64)
                            .unwrap_or(0)
                            .max(0) as u64
                    };
                    restored_cost = restored_cost.saturating_add(micros("cost_usd_micros"));
                    restored_conservative =
                        restored_conservative.saturating_add(micros("conservative_cost_usd_micros"));
                }
            }
        }
        Ok(InnerLoop {
            kernel,
            writer: outcome.writer,
            stream_id,
            log_root: log_root.to_path_buf(),
            feedback_injection,
            max_steps,
            cost_total_micros: restored_cost,
            conservative_cost_total_micros: restored_conservative,
            mission_cost_start: restored_cost,
            mission_conservative_start: restored_conservative,
            budget_micros: None,
            tools: None,
            model_override: None,
            pending_children: Vec::new(),
            swarm_depth: std::env::var("HS_SWARM_DEPTH")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            progress_path: None,
            ledger: Default::default(),
            context_budget_chars: DEFAULT_CONTEXT_BUDGET_TOKENS * 4,
            memory_store: None,
            goal: None,
            dead_tools: Default::default(),
            doom_nudges: Default::default(),
            guardrail_escalator: Default::default(),
            verifier_rounds: 0,
            completion_mode: None,
            idi_interventions: 0,
            idi_detections: 0,
            idi_repairs: 0,
            prior_gaps: vec![],
            wall_secs: None,
            steering_inbox: None,
            task_inbox: None,
            queued_tasks: std::collections::VecDeque::new(),
            interrupt_file: None,
            mission_started: None,
            ui_sink: None,
            last_model: None,
            prefetch: None,
            prefetch_hits: 0,
            prefetch_misses: 0,
            prefetch_enabled: true,
            prefetch_min_samples: PREFETCH_MIN_SAMPLES,
            prefetch_cost_crossover: PREFETCH_COST_CROSSOVER,
            distill_floor,
            world: None,
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

    /// Cumulative CONSERVATIVE list-rate spend (micro-USD) - the figure
    /// budget guards bind (D5). Always >= the provider-reported total.
    #[must_use]
    pub fn conservative_cost_total_micros(&self) -> u64 {
        self.conservative_cost_total_micros
    }

    /// Hard per-mission USD budget (micro-USD). When cumulative provider-reported
    /// cost would exceed the cap, the mission is killed and scored as failed.
    pub fn set_budget_micros(&mut self, micros: u64) {
        self.budget_micros = Some(micros);
    }

    /// The armed USD budget (micro-USD), `None` when nothing bound one
    /// (D4: every session constructor binds one - `None` is only for
    /// harness binaries that deliberately run uncapped before binding).
    #[must_use]
    pub fn budget_micros(&self) -> Option<u64> {
        self.budget_micros
    }

    /// Fix 4 (ab2): the mission's wall budget in seconds. The runner enforces
    /// it externally; this makes it VISIBLE to the model every step.
    /// Live step cap (Eric 2026-09-10: /caps changes it mid-session).
    #[must_use]
    pub fn max_steps(&self) -> u32 {
        self.max_steps
    }

    /// Change the step cap mid-session; the next mission's loop range
    /// uses it (a running loop keeps the range it started with).
    pub fn set_max_steps(&mut self, steps: u32) {
        self.max_steps = steps.max(1);
    }

    /// The armed wall-clock cap in seconds, if any.
    #[must_use]
    pub fn wall_secs(&self) -> Option<u64> {
        self.wall_secs
    }

    /// Disarm the wall-clock cap (/caps wall off).
    pub fn clear_wall_secs(&mut self) {
        self.wall_secs = None;
    }

    pub fn set_wall_secs(&mut self, secs: u64) {
        self.wall_secs = Some(secs);
    }

    /// Gap #3: register a streaming-delta sink on the kernel - model
    /// output then surfaces incrementally (see `hs_kernel::DeltaSink`).
    pub fn set_delta_sink(&mut self, sink: hs_kernel::DeltaSink) {
        self.kernel.set_delta_sink(sink);
    }

    /// UI batch 1: register the typed UI-event sink (REPL painter).
    pub fn set_ui_sink(&mut self, sink: uipaint::UiSink) {
        self.ui_sink = Some(sink);
    }

    /// Gap #2: point the loop at the operator's steering inbox. Drained at
    /// every step boundary; each line lands in the volatile tail as a
    /// STEERING section (a fresh user-turn section, never a rewritten
    /// earlier message - KV-cache discipline).
    pub fn set_steering_inbox(&mut self, path: &Path) {
        self.steering_inbox = Some(path.to_path_buf());
    }

    /// B3: point the loop at the gateway task inbox. Drained at every
    /// step boundary; each line is a NEW GOAL added mid-run - booked as
    /// gateway traffic and queued; it never rewrites the running
    /// mission's plan (that is what steering is for).
    pub fn set_task_inbox(&mut self, path: &Path) {
        self.task_inbox = Some(path.to_path_buf());
    }

    /// Goals queued by mid-run gateway adds, in injection order.
    pub fn take_queued_goals(&mut self) -> Vec<String> {
        self.queued_tasks.drain(..).collect()
    }

    fn drain_task_inbox(&mut self) -> Vec<String> {
        let Some(path) = &self.task_inbox else {
            return vec![];
        };
        let Ok(body) = std::fs::read_to_string(path) else {
            return vec![];
        };
        let lines: Vec<String> = body
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(std::string::ToString::to_string)
            .collect();
        let _ = std::fs::remove_file(path);
        lines
    }

    /// Drain the gateway task inbox at a step boundary: book each added
    /// goal as Message (kind 12, the spec's gateway-traffic kind) on
    /// this stream and queue it for after the mission closes.
    fn poll_gateway_tasks(&mut self) {
        for goal in self.drain_task_inbox() {
            // serde, never format!-templated JSON: a goal carrying a
            // control byte or quote must book a parseable payload
            // (debug `{:?}` escapes are not JSON escapes).
            let body = serde_json::to_string(&serde_json::json!({
                "gateway": "task_queued",
                "goal": goal,
            }))
            .expect("json! values serialize");
            let _ = self.writer.append(
                hs_core::EventBuilder::new(hs_core::EventKind::Message)
                    .payload(hs_core::Payload::Inline(body.into_bytes())),
            );
            self.queued_tasks.push_back(goal);
        }
    }

    /// Gap #2: point the loop at the operator's interrupt flag file. When
    /// it exists at a step boundary the mission stops with outcome
    /// "interrupted" - artifacts and ledger booked, not an error.
    pub fn set_interrupt_file(&mut self, path: &Path) {
        self.interrupt_file = Some(path.to_path_buf());
    }

    fn drain_steering(&mut self) -> Vec<String> {
        let Some(path) = &self.steering_inbox else {
            return vec![];
        };
        let Ok(body) = std::fs::read_to_string(path) else {
            return vec![];
        };
        let lines: Vec<String> = body
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(std::string::ToString::to_string)
            .collect();
        // drained, not re-read: remove so a later step cannot re-consume
        let _ = std::fs::remove_file(path);
        lines
    }

    fn interrupt_requested(&self) -> bool {
        self.interrupt_file
            .as_ref()
            .is_some_and(|p| p.exists())
    }

    /// Native tool schemas for the operator model call (builtin +
    /// MCP-discovered), delivered via the provider API's tools parameter.
    pub fn set_tools(&mut self, tools: serde_json::Value) {
        self.tools = Some(tools);
    }

    /// Eric's five #4: override which configured model serves
    /// operator calls from the next call onward. None restores the
    /// config default; an unknown name is rejected, naming it.
    /// Async delegation: a child loop's depth is its parent's + 1,
    /// set by the spawner before the mission runs.
    pub fn set_swarm_depth(&mut self, depth: u32) {
        self.swarm_depth = depth;
    }

    /// 7.5: overlay the promoted prefetch policy's retirement knobs
    /// (basis points, the policy-layer representation). Defaults remain
    /// the compiled-in constants until a promotion overlays them.
    pub fn set_prefetch_knobs(&mut self, min_samples: u32, cost_crossover_bp: u32) {
        self.prefetch_min_samples = min_samples;
        self.prefetch_cost_crossover = f64::from(cost_crossover_bp) / 10_000.0;
    }

    pub fn set_model_override(&mut self, model: Option<String>) -> Result<(), LoopError> {
        if let Some(m) = &model
            && !self.kernel.has_model(m) {
                return Err(LoopError::Visibility(format!("unknown model: {m}")));
            }
        // Spec 2.7/9.11: capability swaps are log transactions, not
        // silent edits. Book the binding change on the session stream
        // before it takes effect: old effective model -> new effective
        // model (None restores the config default, named explicitly).
        let old_model = self.effective_model_name();
        self.model_override = model;
        let new_model = self.effective_model_name();
        if old_model != new_model {
            // serde, never format!-templated JSON: a model name with a
            // quote or backslash must book a parseable payload.
            let body = serde_json::to_string(&serde_json::json!({
                "capability": "model",
                "old_binding": old_model,
                "new_binding": new_model,
            }))
            .expect("json! values serialize");
            self.writer.append(
                hs_core::EventBuilder::new(hs_core::EventKind::CapabilityChange)
                    .payload(hs_core::Payload::Inline(body.into_bytes())),
            )?;
        }
        Ok(())
    }

    /// The model the next call would actually use: the override when
    /// set, otherwise the config default.
    fn effective_model_name(&self) -> String {
        match &self.model_override {
            Some(m) => m.clone(),
            None => self
                .kernel
                .model_names()
                .into_iter()
                .find(|(_, d)| *d)
                .map(|(n, _)| n)
                .unwrap_or_default(),
        }
    }

    /// Configured models as (name, `is_default`) for the picker.
    pub fn model_names(&self) -> Vec<(String, bool)> {
        self.kernel.model_names()
    }

    /// The native schemas currently delivered on operator model calls
    /// (None = free-form path; REPL parity gap #1 made Some the REPL norm).
    pub fn native_tools(&self) -> Option<&serde_json::Value> {
        self.tools.as_ref()
    }

    /// Wall-kill resilience (phase 1, design D6): when set, the loop writes
    /// a JSON checkpoint of {steps, `model_calls`, `cost_micros`} after EVERY
    /// step. An external wall-clock kill (timeout, OOM, SIGKILL) then books
    /// from the checkpoint via `book_wall_kill` instead of writing a
    /// 0-step result for a run that did real work.
    /// B1 (v5 D3 + cut #10): attach the typed memory plane K. The model
    /// consults it through the `memory.recall` tool - K is never pre-passed
    /// into prompts; every mission close distills its trajectory into K.
    pub fn set_memory_db(&mut self, path: &Path) {
        self.memory_store = Some(Box::new(
            hs_memory::sqlite::SqliteMemoryStore::open(path).expect("memory db open"),
        ));
    }

    /// D6: acceptance-constrained stopping. The stop decision becomes a
    /// verifiable predicate (patch applies + F2P green in the sandbox).
    pub fn set_goal_evaluator(&mut self, ws: &Path, f2p: Vec<String>) {
        self.goal = Some(goal::GoalSpec {
            ws: ws.to_path_buf(),
            f2p,
            timeout_secs: 120,
        });
    }

    /// D1: size the transcript projection in tokens (4 chars/token proxy).
    pub fn set_context_budget_tokens(&mut self, tokens: usize) {
        self.context_budget_chars = tokens.saturating_mul(4);
    }

    pub fn set_progress_path(&mut self, path: &Path) {
        self.progress_path = Some(path.to_path_buf());
    }

    /// Fix 4: the wall guard. A wall-killed mission is a failure,
    /// same as a budget-killed one. Booking here keeps the evidence:
    /// the mission exits cleanly, the runner pulls the run dir, and
    /// the official verifier still grades the final machine state.
    /// Called from the step loop AND the passing-close join loop -
    /// before the wall-guard follow-up the join loop had no wall check,
    /// so a wedged child (plugin thread stuck, report never written)
    /// hung the parent past its own guard (hostile-pass finding,
    /// 2026-09-08). Returns Some(result) when the wall was exceeded.
    fn wall_kill_close(
        &mut self,
        mission: &str,
        steps: u32,
        model_calls: u32,
        answer_path: &std::path::Path,
    ) -> Result<Option<MissionResult>, LoopError> {
        if let (Some(w), Some(t0)) = (self.wall_secs, self.mission_started) {
            let elapsed = t0.elapsed().as_secs();
            if elapsed >= w {
                self.writer.append(
                    EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                        serde_json::to_vec(&serde_json::json!({
                            "wall_killed": true, "wall_secs": w,
                            "elapsed_secs": elapsed,
                            "cost_micros": self.cost_total_micros,
                        }))
                        .expect("json! values serialize"),
                    )),
                )?;
                self.checkpoint(steps, model_calls);
                self.close_goal(mission, false, "wall_killed")?;
                return Ok(Some(MissionResult {
                    passed: false,
                    steps,
                    model_calls,
                    stream_id: self.stream_id,
                    answer_path: answer_path.to_path_buf(),
                    budget_killed: false,
                    cost_micros: self
                        .cost_total_micros
                        .saturating_sub(self.mission_cost_start),
                    conservative_cost_micros: self
                        .conservative_cost_total_micros
                        .saturating_sub(self.mission_conservative_start),
                    harness_error: None,
                    outcome: "wall_killed".to_string(),
                }));
            }
        }
        Ok(None)
    }

    fn checkpoint(&self, steps: u32, model_calls: u32) {
        if let Some(p) = &self.progress_path {
            let body = serde_json::json!({
                "steps": steps,
                "model_calls": model_calls,
                "cost_micros": self.cost_total_micros,
                "conservative_cost_micros": self.conservative_cost_total_micros,
            });
            let _ = std::fs::write(
                p,
                serde_json::to_string(&body).expect("json! values serialize"),
            );
        }
    }

    /// Answer-path tools whose death is mission-terminal (measurement run
    /// ab2/17123): without edit.patch/answer.submit no mission can land, so
    /// continuing burns steps for nothing. Death of any OTHER tool degrades
    /// to feedback instead of aborting - the mission continues while the
    /// answer path remains usable.
    fn is_answer_path(tool: &str) -> bool {
        matches!(
            tool,
            "answer.submit" | "edit.patch" | "edit.anchor" | "answer.write" | "edit.apply"
        )
    }

    /// Abort the mission on a supervisor-declared dead ANSWER-PATH plugin:
    /// book the `harness_error` as a Feedback event (trace-visible) and return
    /// the partial result. This replaces the old behavior of feeding the
    /// error back and burning the remaining steps against a dead plugin (run
    /// 17117 lost ~24 calls that way).
    /// M12: every mission END closes the goal on the durable stream -
    /// success or failure. The resume picker's operator-stream marker
    /// (Feedback|GoalUpdate) and the delegation graph's completion read
    /// both depend on it; before M12 only the checker-green stop path
    /// wrote `GoalUpdate`, so plain REPL sessions were invisible to
    /// `:resume` (live-proof cap5, 2026-09-08: operator stream kinds
    /// [0,0,0,0] after two completed missions). `outcome` names the
    /// ending; hs-swarm's spawn-time `GoalUpdate` {done:false} carries
    /// no outcome, so an OPEN goal is never mistaken for a failed one.
    /// B1 (v5 D3 + cut #10): tool dispatch. D3's K plane is consulted by
    /// the model as a builtin tool - it never reaches the plugin bus and
    /// is never pre-passed into prompts.
    /// The session's mission workdir (`log_root/work/<mission-id>`): the
    /// sandbox filesystem state recovery tiers B and C restore. The
    /// substrate (streams + blobs) is durable separately and never
    /// snapshotted.
    #[must_use]
    pub fn work_dir(&self) -> std::path::PathBuf {
        self.log_root.join("work")
    }

    /// Recovery tier B (spec v5 "Recovery tiers"): snapshot the mission
    /// workdir into the content-addressed store. `World::snapshot` books
    /// the `SnapshotRef` on the world stream; a `SnapshotRef` also lands on
    /// the MISSION stream so the mission's own evidence chain names the
    /// state it can be restored from.
    pub fn snapshot_workdir(&mut self) -> Result<hs_world::SnapshotReport, LoopError> {
        if self.world.is_none() {
            self.attach_world();
        }
        let rep = self
            .world
            .as_ref()
            .expect("attach_world just ran")
            .snapshot(&self.work_dir())
            .map_err(|e| LoopError::World(format!("{e:?}")))?;
        self.writer.append(
            EventBuilder::new(EventKind::SnapshotRef).payload(Payload::Inline(
                serde_json::to_vec(&serde_json::json!({
                    "snapshot_id": rep.snapshot_id,
                    "tier": "B",
                    "files": rep.files,
                    "bytes": rep.bytes,
                    "took_ms": rep.took_ms,
                }))
                .expect("json! values serialize"),
            )),
        )?;
        Ok(rep)
    }

    /// Recovery tier B restore: hash-verified, byte-exact rehydration of
    /// the mission workdir ("process and filesystem state back"), with the
    /// recovery BOOKED on the mission stream and measured - B6b's
    /// `T_mission` R term reads this, it is never assumed.
    pub fn restore_workdir(&mut self, snapshot_id: &str) -> Result<hs_world::SnapshotReport, LoopError> {
        if self.world.is_none() {
            self.attach_world();
        }
        let t0 = std::time::Instant::now();
        let rep = self
            .world
            .as_ref()
            .expect("attach_world just ran")
            .restore(snapshot_id, &self.work_dir())
            .map_err(|e| LoopError::World(format!("{e:?}")))?;
        let duration_ms = t0.elapsed().as_millis();
        self.writer.append(
            EventBuilder::new(EventKind::Observation).payload(Payload::Inline(
                format!(
                    "recovery tier=B restore snapshot_id={} files={} duration_ms={duration_ms}",
                    rep.snapshot_id, rep.files
                )
                .into_bytes(),
            )),
        )?;
        Ok(rep)
    }

    /// B2 (v5 gate 6): join the shared world plane (one world stream in
    /// this substrate). Called by every REPL session; missions then reach
    /// the world through the world.* tools below.
    pub fn attach_world(&mut self) {
        self.world = Some(
            hs_world::World::open(&self.log_root).expect("world open at the session log root"),
        );
    }

    /// The world plane this session joined (test/admin reach: ticks,
    /// installs, uninstalls are world operations, not model tools).
    #[must_use]
    pub fn world(&self) -> Option<&hs_world::World> {
        self.world.as_ref()
    }

    /// The canonical world stream (proposal/consequence evidence reads).
    #[must_use]
    pub fn world_stream_id(&self) -> uuid::Uuid {
        self.world
            .as_ref()
            .expect("attach_world first")
            .world_stream()
    }

    /// B2 (spec 3.4): world tool dispatch. The agent writes proposals; the
    /// world service alone validates (schema, hash, path, version,
    /// quarantine) and writes the consequence. Rejections are served to
    /// the model as ordinary tool output - never fatal to the mission.
    /// Returns None for non-world tools.
    fn world_dispatch(&mut self, tool: &str, args: &serde_json::Value) -> Option<ToolCallOutcome> {
        if !matches!(tool, "world.propose" | "world.observe" | "world.install" | "world.tick") {
            return None;
        }
        let started = std::time::Instant::now();
        let done = |output: serde_json::Value| {
            Some(ToolCallOutcome {
                resolved: None,
                output,
                latency_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
            })
        };
        let Some(world) = &self.world else {
            return done(serde_json::json!({"error": format!("{tool}: no world attached to this session")}));
        };
        match tool {
            "world.propose" => {
                use sha2::Digest as _;
                let path = args["world_path"].as_str().unwrap_or("").to_string();
                let content = args["content"].as_str().unwrap_or("").to_string();
                let kind = match args["kind"].as_str().unwrap_or("file") {
                    "program" => hs_world::ArtifactKind::Program,
                    "controller" => hs_world::ArtifactKind::Controller,
                    "note" => hs_world::ArtifactKind::Note,
                    "skill" => hs_world::ArtifactKind::Skill,
                    _ => hs_world::ArtifactKind::File,
                };
                let artifact = hs_world::Artifact {
                    artifact_id: args["artifact_id"]
                        .as_str()
                        .and_then(|s| uuid::Uuid::parse_str(s).ok())
                        .unwrap_or_else(uuid::Uuid::new_v4),
                    version: u32::try_from(args["version"].as_u64().unwrap_or(1)).unwrap_or(1),
                    kind,
                    content_hash: sha2::Sha256::digest(content.as_bytes()).into(),
                    world_path: path,
                    author_stream: self.stream_id,
                    parent_version: match args.get("parent_version") {
                        None | Some(serde_json::Value::Null) => None,
                        Some(v) => match v.as_str().and_then(|s| uuid::Uuid::parse_str(s).ok()) {
                            some @ Some(_) => some,
                            None => {
                                return done(serde_json::json!({
                                    "error": "world.propose: parent_version must be a uuid string"
                                }));
                            }
                        },
                    },
                    status: hs_world::ArtifactStatus::Proposed,
                };
                match world.propose(artifact, content.as_bytes()) {
                    Ok(a) => done(serde_json::json!({
                        "artifact_id": a.artifact_id, "version": a.version,
                        "kind": a.kind, "world_path": a.world_path,
                        "author_stream": a.author_stream, "status": "validated",
                    })),
                    Err(e) => done(serde_json::json!({
                        "error": format!("world rejected the proposal: {e:?}")
                    })),
                }
            }
            "world.observe" => {
                let path = args["world_path"].as_str().unwrap_or("");
                match world.observe_as(self.stream_id, path) {
                    Ok(arts) => {
                        let rows: Vec<serde_json::Value> = arts
                            .iter()
                            .map(|a| {
                                serde_json::json!({
                                    "artifact_id": a.artifact_id, "version": a.version,
                                    "kind": a.kind, "world_path": a.world_path,
                                    "author_stream": a.author_stream, "status": a.status,
                                    "content_hash": a.content_hash,
                                    "parent_version": a.parent_version,
                                    "reuse_count": world.reuse_count(a.artifact_id, a.version),
                                })
                            })
                            .collect();
                        done(serde_json::json!({"world_path": path, "count": rows.len(), "artifacts": rows}))
                    }
                    Err(e) => done(serde_json::json!({"error": format!("world.observe: {e:?}")})),
                }
            }
            "world.install" => {
                let Some(id) = args["artifact_id"]
                    .as_str()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                else {
                    return done(serde_json::json!({"error": "world.install: artifact_id must be a uuid"}));
                };
                match world.install(id) {
                    Ok(()) => done(serde_json::json!({"installed": id.to_string()})),
                    Err(e) => done(serde_json::json!({"error": format!("world.install: {e:?}")})),
                }
            }
            _ => match world.tick() {
                Ok(ids) => done(serde_json::json!({
                    "acted": ids.iter().map(std::string::ToString::to_string).collect::<Vec<_>>(),
                    "count": ids.len(),
                })),
                Err(e) => done(serde_json::json!({"error": format!("world.tick: {e:?}")})),
            },
        }
    }

    /// Spec 4.4 on the mission path (burn-down item 4): the mission's
    /// artifact write - answer.write - rides the proposal-consequence
    /// separation like any world effect. The agent only ever writes a
    /// PROPOSAL event; the world service alone validates (schema, hash,
    /// quarantine status of the proposing stream) and the CONSEQUENCE is
    /// what materializes the file the checker grades. Rejections are
    /// ordinary tool output, never fatal. Sessions with no world plane
    /// (crate-level loop tests) keep the direct plugin path.
    ///
    /// The sandbox plane (term.exec) is the execution environment
    /// itself, not an artifact effect - spec section 4: "for software
    /// work, the world is the execution environment" - so it is not
    /// routed here; artifact effects are.
    fn world_artifact_write(&mut self, tool: &str, args: &serde_json::Value) -> Option<ToolCallOutcome> {
        if tool != "answer.write" && tool != "answer.submit" {
            return None;
        }
        let world = self.world.as_ref()?;
        let started = std::time::Instant::now();
        // Compute the submission CONTENT on the agent side (hash input for
        // the proposal); the EFFECT belongs to the world alone. INVALID
        // submissions fall through (None) to the real plugin: its own
        // $error then flows through the kernel's normal error arm (booked
        // as a tool error, feedback, no artifact to grade) - only the
        // happy path changes hands, so a REJECTED submit can never look
        // like a graded one (critic_tui_red close-accounting burn,
        // 2026-09-10).
        let (content, submit_shape) = if tool == "answer.write" {
            (args["content"].as_str().unwrap_or("").to_string(), false)
        } else {
            let path = args["path"].as_str().unwrap_or("");
            if path.is_empty() {
                return None;
            }
            if std::env::var("HS_ANSWER_RAW").as_deref() == Ok("1") {
                let summary = args["summary"].as_str().unwrap_or("").to_string();
                if summary.trim().is_empty() {
                    return None;
                }
                (summary, true)
            } else {
                let Ok(ws) = std::env::var("HS_SWE_WORKSPACE") else {
                    return None;
                };
                match crate::editapply::answer_diff_text(std::path::Path::new(&ws)) {
                    Ok(d) => (d, true),
                    Err(_) => return None,
                }
            }
        };
        let path = args["path"].as_str().unwrap_or("").to_string();
        use sha2::Digest as _;
        let artifact = hs_world::Artifact {
            artifact_id: uuid::Uuid::new_v4(),
            version: 1,
            kind: hs_world::ArtifactKind::File,
            content_hash: sha2::Sha256::digest(content.as_bytes()).into(),
            world_path: path,
            author_stream: self.stream_id,
            parent_version: None,
            status: hs_world::ArtifactStatus::Proposed,
        };
        let output = match world.propose(artifact, content.as_bytes()) {
            Err(e) => serde_json::json!({"error": format!("world rejected the proposal: {e:?}")}),
            Ok(a) => match world.materialize(&a) {
                Ok(p) => {
                    if submit_shape {
                        serde_json::json!({"written": true, "path": p.display().to_string(), "bytes": content.len()})
                    } else {
                        serde_json::json!({"path": p.display().to_string(), "written": true})
                    }
                }
                Err(e) => serde_json::json!({"error": format!("world materialization failed: {e:?}")}),
            },
        };
        Some(ToolCallOutcome {
            resolved: None,
            output,
            latency_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
        })
    }

    fn dispatch_tool(
        &mut self,
        tool: &str,
        args: &serde_json::Value,
    ) -> Result<ToolCallOutcome, KernelError> {
        if tool == "memory.recall" {
            return Ok(self.memory_recall(args));
        }
        if let Some(out) = self.world_dispatch(tool, args) {
            return Ok(out);
        }
        if let Some(out) = self.world_artifact_write(tool, args) {
            return Ok(out);
        }
        self.kernel.call_tool("operator", tool, args.clone())
    }

    /// B1: serve one `memory.recall`. Resolution order: evaluate the
    /// outstanding prefetch (identical args = the speculation hit and the
    /// result is reused), serve from K, then - while the predictor is still
    /// running - speculatively pre-fetch the predicted next query (the
    /// last-query predictor) so it rides this call's tail and not the
    /// next model call's round trip. Every resolved speculation is booked
    /// with hit/miss, its estimated token cost, and the predictor's
    /// retirement flag.
    fn memory_recall(&mut self, args: &serde_json::Value) -> ToolCallOutcome {
        let started = std::time::Instant::now();
        if self.memory_store.is_none() {
            return ToolCallOutcome {
                resolved: None,
                output: serde_json::json!({
                    "error": "memory.recall: no memory store attached to this session"
                }),
                latency_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
            };
        }
        let k = usize::try_from(
            args.get("k")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(5)
                .clamp(1, 50),
        )
        .unwrap_or(5);
        let want = serde_json::json!({"k": k});
        let mut cached: Option<serde_json::Value> = None;
        if let Some(pref) = self.prefetch.take() {
            let hit = pref.args == want;
            if hit {
                self.prefetch_hits += 1;
                cached = Some(pref.result);
            } else {
                self.prefetch_misses += 1;
            }
            let total = self.prefetch_hits + self.prefetch_misses;
            let retired = self.prefetch_enabled
                && total >= self.prefetch_min_samples
                && (f64::from(self.prefetch_hits) / f64::from(total))
                    < self.prefetch_cost_crossover;
            if retired {
                self.prefetch_enabled = false;
            }
            let _ = self.writer.append(
                EventBuilder::new(EventKind::Prefetch).payload(Payload::Inline(
                    serde_json::to_vec(&serde_json::json!({
                        "predicted": pref.args, "hit": hit,
                        "tokens_est": pref.tokens_est,
                        "hits": self.prefetch_hits, "misses": self.prefetch_misses,
                        "crossover": self.prefetch_cost_crossover,
                        "min_samples": self.prefetch_min_samples,
                        "predictor_retired": retired,
                    }))
                    .expect("json! values serialize"),
                )),
            );
        }
        let output = cached.unwrap_or_else(|| self.fetch_memory(k));
        if self.prefetch_enabled && output.get("error").is_none() {
            let tokens_est =
                u32::try_from(output.to_string().len().div_ceil(4)).unwrap_or(u32::MAX).max(1);
            self.prefetch = Some(PrefetchCache {
                args: want,
                result: output.clone(),
                tokens_est,
            });
        }
        ToolCallOutcome {
            resolved: None,
            output,
            latency_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
        }
    }

    /// B1: read-side of the K query - top-k by importance then recency
    /// (the `MemoryStore` contract), served as records with seq refs so
    /// the model can quote provenance.
    fn fetch_memory(&self, k: usize) -> serde_json::Value {
        let Some(store) = &self.memory_store else {
            return serde_json::json!({"error": "memory.recall: no memory store attached to this session"});
        };
        match store.top_k("operator", k) {
            Ok(recs) => {
                let rows: Vec<serde_json::Value> = recs
                    .iter()
                    .map(|r| {
                        serde_json::json!({
                            "id": r.id, "kind": r.kind, "agent_id": r.agent_id,
                            "mission_id": r.mission_id, "content": r.content,
                            "importance": r.importance, "expires_at": r.expires_at,
                            "source_seqs": r.source_seqs, "created_at": r.created_at,
                        })
                    })
                    .collect();
                serde_json::json!({"count": rows.len(), "records": rows})
            }
            Err(e) => serde_json::json!({"error": format!("memory.recall: {e}")}),
        }
    }

    fn close_goal(&mut self, mission: &str, done: bool, outcome: &str) -> Result<(), LoopError> {
        // B1: close-side K bookkeeping runs BEFORE the terminal
        // GoalUpdate, which must stay the mission's last event (M12:
        // the resume picker and the delegation graph read the LAST
        // event as the goal's state).
        // B1: an unresolved speculation at close is speculative work the
        // next model call never cashed - resolve it as not-consumed (a
        // miss in the predictor's books) with its cost visible.
        if let Some(pref) = self.prefetch.take() {
            self.prefetch_misses += 1;
            self.writer.append(
                EventBuilder::new(EventKind::Prefetch).payload(Payload::Inline(
                    serde_json::to_vec(&serde_json::json!({
                        "predicted": pref.args, "hit": false, "consumed": false,
                        "tokens_est": pref.tokens_est,
                        "hits": self.prefetch_hits, "misses": self.prefetch_misses,
                    }))
                    .expect("json! values serialize"),
                )),
            )?;
        }
        // B1 (v5 close contract): EVERY mission close distills its
        // trajectory into K - deterministic extraction today, the LLM
        // handoff distiller plugs into the same provenance contract
        // later. The booking makes the write auditable from the log.
        if self.memory_store.is_some() {
            let records = hs_memory::extract::extract_stream_from(
                &self.log_root,
                self.stream_id,
                mission,
                "operator",
                self.distill_floor,
            );
            // SwarmWorld fidelity (gap 2): the distiller's output also
            // flows into the world as culture - procedural records as
            // skills, everything else as notes. Proposals are validated
            // by the world service and land VALIDATED; the loop NEVER
            // installs - installation stays an explicit world.install
            // decision (the assay gate; spec promotion machinery is
            // future work).
            let mut world_proposed = 0usize;
            if let Some(world) = &self.world {
                use sha2::Digest as _;
                let slug: String = mission
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '-' {
                            c
                        } else {
                            '-'
                        }
                    })
                    .collect();
                for r in &records {
                    let (kind, prefix) = if r.kind == "procedural" {
                        (hs_world::ArtifactKind::Skill, "/skills")
                    } else {
                        (hs_world::ArtifactKind::Note, "/knowledge")
                    };
                    let content = r.content.clone();
                    let artifact = hs_world::Artifact {
                        artifact_id: uuid::Uuid::new_v4(),
                        version: 1,
                        kind,
                        content_hash: sha2::Sha256::digest(content.as_bytes()).into(),
                        world_path: format!("{prefix}/{slug}"),
                        author_stream: self.stream_id,
                        parent_version: None,
                        status: hs_world::ArtifactStatus::Proposed,
                    };
                    if world.propose(artifact, content.as_bytes()).is_ok() {
                        world_proposed += 1;
                    }
                }
            }
            let mut written = 0usize;
            if let Some(store) = &self.memory_store {
                for r in records {
                    if store.put(r).is_ok() {
                        written += 1;
                    }
                }
            }
            self.writer.append(
                EventBuilder::new(EventKind::Observation).payload(Payload::Inline(
                    serde_json::to_vec(&serde_json::json!({
                        "memory_distilled": written, "mission": mission,
                        "outcome": outcome,
                        "world_proposed": world_proposed,
                    }))
                    .expect("json! values serialize"),
                )),
            )?;
        }
        let gu = self.writer.append(
            EventBuilder::new(EventKind::GoalUpdate).payload(Payload::Inline(
                serde_json::to_vec(&serde_json::json!({
                    "mission": mission, "done": done, "outcome": outcome,
                    "completion_mode": self.completion_mode.unwrap_or("none"),
                    "interventions": self.idi_interventions,
                    "detections": self.idi_detections,
                    "repairs": self.idi_repairs,
                    "repair_rate": if self.idi_detections > 0 {
                        serde_json::Value::from(self.idi_repairs as f64 / self.idi_detections as f64)
                    } else {
                        serde_json::Value::Null
                    },
                }))
                .expect("json! values serialize"),
            )),
        )?;
        // the next close slices above this terminal event
        self.distill_floor = gu.seq;
        Ok(())
    }

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
                .expect("json! values serialize"),
            )),
        )?;
        self.close_goal(mission, false, "harness_error")?;
        Ok(MissionResult {
            passed: false,
            steps,
            model_calls,
            stream_id: self.stream_id,
            answer_path: answer_path.to_path_buf(),
            budget_killed: false,
            cost_micros: self.cost_total_micros.saturating_sub(self.mission_cost_start),
            conservative_cost_micros: self
                .conservative_cost_total_micros
                .saturating_sub(self.mission_conservative_start),
            harness_error: Some(msg),
            outcome: "harness_error".to_string(),
        })
    }

    /// Run one mission to a checker verdict, the step cap, or the budget cap.
    /// Poll every running child once (quiet query - never booked).
    /// Finished children fold their cost into this mission's books,
    /// emit `SubAgentFinished` for the panel, and return an ungated
    /// delegation update for the operator's next prompt.
    fn poll_children(&mut self) -> Vec<String> {
        let mut updates = Vec::new();
        let mut i = 0;
        while i < self.pending_children.len() {
            let child = self.pending_children[i].child;
            let out = self.kernel.query_tool(
                "operator",
                "agent.spawn_poll",
                serde_json::json!({"child_stream_id": child.to_string()}),
            );
            match out {
                Ok(v) if v["status"].as_str() == Some("done") => {
                    let pc = self.pending_children.remove(i);
                    let ok = v["passed"].as_bool().unwrap_or(false);
                    if let Some(c) = v["cost_usd_micros"].as_i64()
                        && c > 0 {
                            self.cost_total_micros =
                                self.cost_total_micros.saturating_add(c as u64);
                        }
                    if let Some(sink) = self.ui_sink.as_mut() {
                        sink(uipaint::UiEvent::SubAgentFinished { child, ok });
                    }
                    let short: String = child.to_string().chars().take(8).collect();
                    updates.push(format!(
                        "child {short} (\"{}\") finished - passed={ok}, steps={}, cost ${:.4}; its stream holds the full record",
                        pc.mission,
                        v["steps"].as_i64().unwrap_or(0),
                        v["cost_usd_micros"].as_i64().unwrap_or(0) as f64 / 1e6
                    ));
                }
                // A child the plugin reports LOST died with the plugin
                // process that owned it (marker predates the process) -
                // book the honest failure NOW instead of burning every
                // remaining step and the wall guard on a corpse.
                Ok(v) if v["status"].as_str() == Some("lost") => {
                    let pc = self.pending_children.remove(i);
                    if let Some(sink) = self.ui_sink.as_mut() {
                        sink(uipaint::UiEvent::SubAgentFinished { child, ok: false });
                    }
                    let short: String = child.to_string().chars().take(8).collect();
                    updates.push(format!(
                        "child {short} (\"{}\") LOST - its plugin process died mid-run ({}); re-delegate if the work still matters",
                        pc.mission,
                        v["reason"].as_str().unwrap_or("stale registry marker"),
                    ));
                }
                // Still running, or a transient query failure (retried
                // next step; the wall guard bounds a wedged plugin).
                _ => i += 1,
            }
        }
        updates
    }

    pub fn run_mission(&mut self, mission: &str) -> Result<MissionResult, LoopError> {
        self.run_mission_full(mission, mission)
    }

    /// Gate 8: a mission whose PROMPT differs from its id. The id names the
    /// work dir (must be path-safe); the prompt is the full mission text the
    /// model sees (e.g. a SWE-bench problem statement + response contract).
    /// Arm mission memory unconditionally (dance #95, live burn
    /// 2026-09-09, realrun2): the REPL session default (no `--feedback
    /// on`) ran a production mission on the bake-off "baseline" arm -
    /// `feedback_injection=false` gates transcript assembly, the ledger,
    /// convergence pressure and doom-loop nudges (all in
    /// `run_mission_full`), so the model got a 2-message frame every
    /// step, re-discovered the empty workspace 50 times, and wrote
    /// nothing. The baseline arm exists for the experiment binaries
    /// (hs-swe-run/hs-tb-run `--feedback off`); a production mission
    /// without its own memory is unwinnable for any model, so
    /// `ReplSession::run_goal` arms it here. The experiment flag still
    /// constructs the loop with the arm off - only `run_goal` forces it.
    pub fn arm_mission_memory(&mut self) {
        self.feedback_injection = true;
    }

    pub fn run_mission_full(
        &mut self,
        mission_id: &str,
        prompt: &str,
    ) -> Result<MissionResult, LoopError> {
        let mission = mission_id;
        // Deep-pass hostile review (2026-09-09): the mission id names the
        // work dir, and on the swarm child path the id IS the model's
        // untrusted `agent.spawn` mission string (no slug filter runs on
        // that path). A separator or `..` walks the work dir out of the
        // substrate and `create_dir_all` lands it there. Refuse anything
        // but a single safe path component, at the write site.
        if mission.is_empty()
            || mission == "."
            || mission == ".."
            || mission.contains('/')
            || mission.contains('\\')
        {
            return Err(LoopError::Visibility(format!(
                "mission id must be a single safe path component, got {mission:?}"
            )));
        }
        self.mission_started = Some(std::time::Instant::now());
        self.idi_interventions = 0;
        self.idi_detections = 0;
        self.idi_repairs = 0;
        // M21: per-mission spend is the delta from this point.
        self.mission_cost_start = self.cost_total_micros;
        self.mission_conservative_start = self.conservative_cost_total_micros;
        // A stale interrupt flag from a previous mission must not abort
        // THIS one: the flag applies to the mission that was running
        // when it was set. Clear it once at mission start.
        if let Some(p) = &self.interrupt_file {
            let _ = std::fs::remove_file(p);
        }
        let answer_path = self.log_root.join("work").join(mission).join("answer.txt");
        std::fs::create_dir_all(
            answer_path
                .parent()
                .expect("joined path always has a parent"),
        )?;
        let mut pending_feedback: Vec<String> = vec![];
        let mut steps = 0u32;
        let mut model_calls = 0u32;

        for step in 1..=self.max_steps {
            if let Some(sink) = self.ui_sink.as_mut() {
                sink(uipaint::UiEvent::Step {
                    step,
                    max_steps: self.max_steps,
                });
            }
            // Gap #2: operator interrupt at the step boundary. Booked as
            // its own outcome - operator intent, never a harness error,
            // never a pass.
            if self.interrupt_requested() {
                let done = step - 1;
                let _ = self.writer.append(
                    EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                        serde_json::to_vec(&serde_json::json!({
                            "interrupted": true, "after_steps": done,
                        }))
                        .expect("json! values serialize"),
                    )),
                );
                self.checkpoint(done, model_calls);
                let _ = self.close_goal(mission, false, "interrupted");
                return Ok(MissionResult {
                    passed: false,
                    steps: done,
                    model_calls,
                    stream_id: self.stream_id,
                    answer_path,
                    budget_killed: false,
            cost_micros: self.cost_total_micros.saturating_sub(self.mission_cost_start),
            conservative_cost_micros: self
                .conservative_cost_total_micros
                .saturating_sub(self.mission_conservative_start),
                    harness_error: None,
                    outcome: "interrupted".to_string(),
                });
            }
            steps = step;
            // Async delegation: finished children land here, every step.
            let delegation_updates = self.poll_children();
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
                "content": crate::msgfmt::mission_first_message(&prompt),
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
            // Delegation updates are mission events, never gated by
            // feedback mode: the model delegated and owns the outcome.
            if !delegation_updates.is_empty() {
                volatile.push_str("DELEGATION UPDATES (children you spawned):
");
                for u in &delegation_updates {
                    volatile.push_str(&format!("- {u}\n"));
                }
            }
            // Gap #2: drained operator steering rides the volatile tail
            // like FEEDBACK - the operator's fresh mid-mission user turn.
            self.poll_gateway_tasks();
            let steered = self.drain_steering();
            if !steered.is_empty() {
                volatile.push_str(
                    "STEERING (operator, mid-mission; overrides earlier plans where they conflict):\n",
                );
                for s in &steered {
                    volatile.push_str(&format!("- {s}\n"));
                }
            }
            volatile.push_str(&artifact_section(&answer_path, &artifact));
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
                    "CONVERGENCE: step {step} of {} and you have not run a test or check yourself. Verify your current hypothesis NOW: run a test, a build, or the checker before your next edit.",
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
            if self.feedback_injection
                && let Ok(reader) = hs_log::StreamReader::open(&self.log_root, self.stream_id)
                    && let Ok(events) = reader.events() {
                        volatile.push_str("LEDGER (your work so far, always current):\n");
                        volatile.push_str(&self.ledger.summary());
                        let mut asm = assembler::assemble_messages(
                            &reader,
                            &events,
                            self.context_budget_chars,
                        );
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
                            // M18: same bracket for the distill call -
                            // it is booked (model_calls, cost) and must
                            // show on the HUD like any other call.
                            if let Some(sink) = self.ui_sink.as_mut() {
                                sink(uipaint::UiEvent::ModelCallStart {
                                    model: self.last_model.clone().unwrap_or_default(),
                                });
                            }
                            if let Ok(out) =
                                self.kernel.call_model("operator", self.model_override.as_deref(), &distill_prompt)
                            {
                                model_calls += 1;
                                self.cost_total_micros += out.cost_usd_micros.max(0) as u64;
            self.conservative_cost_total_micros += out.conservative_cost_usd_micros.max(0) as u64;
                                if let Some(sink) = self.ui_sink.as_mut() {
                                    sink(uipaint::UiEvent::ModelCallEnd {
                                        model: out.model.clone(),
                                        input_tokens: out.input_tokens,
                                        output_tokens: out.output_tokens,
                                        cost_usd_micros: out.cost_usd_micros,
                                    });
                                }
                                    if let Some(ev) = uipaint::reasoning_event(&out.reasoning_content) {
                                        if let Some(sink) = self.ui_sink.as_mut() {
                                            sink(ev);
                                        }
                                    }
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
                                                    "conservative_cost_usd_micros": out.conservative_cost_usd_micros,
                                                }))
                                                .expect("json! values serialize"),
                                            ))
                                            .latency_ms(out.latency_ms)
                                            .cost_usd_micros(out.cost_usd_micros),
                                    );
                                distilled = Some(out.completion);
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

            messages.push(serde_json::json!({"role": "user", "content": volatile}));
            let assembly_ms = t_assembly.elapsed().as_millis() as u64; // capture BEFORE the model call (was after: read as ~latency)
            let messages = serde_json::Value::Array(messages);

            // M10: bracket the round trip with UI events - the rail and
            // HUD vitals derive from these mid-mission.
            if let Some(sink) = self.ui_sink.as_mut() {
                sink(uipaint::UiEvent::ModelCallStart {
                    model: self.last_model.clone().unwrap_or_default(),
                });
            }
            // the only model round trip in the step
            let out = match self.kernel.call_model_messages(
                "operator",
                self.model_override.as_deref(),
                &messages,
                self.tools.as_ref(),
            ) {
                Ok(o) => o,
                Err(e @ KernelError::PluginApp { .. }) => {
                    // persistent provider failure (the plugin already burned
                    // its own retries): book it, don't strike-loop
                    self.checkpoint(steps, model_calls);
                    return self.abort_harness(
                        mission,
                        &answer_path,
                        steps,
                        model_calls,
                        e.to_string(),
                    );
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
            self.conservative_cost_total_micros += out.conservative_cost_usd_micros.max(0) as u64;
            self.last_model = Some(out.model.clone());
            if let Some(sink) = self.ui_sink.as_mut() {
                sink(uipaint::UiEvent::ModelCallEnd {
                    model: out.model.clone(),
                    input_tokens: out.input_tokens,
                    output_tokens: out.output_tokens,
                    cost_usd_micros: out.cost_usd_micros,
                });
            }
                if let Some(ev) = uipaint::reasoning_event(&out.reasoning_content) {
                    if let Some(sink) = self.ui_sink.as_mut() {
                        sink(ev);
                    }
                }
            // T5c: checkpoint EVERY step after the model-call accounting,
            // answer or not - a wall kill must never book a 0-step row for
            // a mission that did real work (ab2 17092/17102/17117 lost
            // 19-25 steps each to answer-only checkpointing).
            self.checkpoint(steps, model_calls);
            if let Some(cap) = self.budget_micros
                && self.conservative_cost_total_micros > cap {
                    self.writer.append(
                        EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                            serde_json::to_vec(&serde_json::json!({
                                "budget_killed": true, "cap_micros": cap,
                                "cost_micros": self.cost_total_micros,
                                "conservative_cost_micros": self.conservative_cost_total_micros,
                            }))
                            .expect("json! values serialize"),
                        )),
                    )?;
                    self.checkpoint(steps, model_calls);
                    self.close_goal(mission, false, "budget_killed")?;
                    return Ok(MissionResult {
                        passed: false,
                        steps,
                        model_calls,
                        stream_id: self.stream_id,
                        answer_path,
                        budget_killed: true,
            cost_micros: self.cost_total_micros.saturating_sub(self.mission_cost_start),
            conservative_cost_micros: self
                .conservative_cost_total_micros
                .saturating_sub(self.mission_conservative_start),
                        harness_error: None,
                        outcome: "budget_killed".to_string(),
                    });
                }
            // Wall guard fires INSIDE the loop, at the same step boundary
            // as the budget guard. Live burn 2026-09-07 (glm-critic TB
            // trial): enforcement had been delegated to the harness's
            // external exec timeout (wall + 600s grace), which killed the
            // container 10 minutes past the guard and destroyed every
            // artifact - ledger, critic trace, checks, answer. Booking
            // here keeps the evidence: the mission exits cleanly, the
            // runner pulls the run dir, and the official verifier still
            // grades the final machine state. A wall-killed mission is a
            // failure, same as a budget-killed one.
            if let Some(res) = self.wall_kill_close(mission, steps, model_calls, &answer_path)? {
                return Ok(res);
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
                            "cost_usd_micros": out.cost_usd_micros,
                            "conservative_cost_usd_micros": out.conservative_cost_usd_micros,
                            "assembly_ms": assembly_ms,
                        }))
                        .expect("json! values serialize"),
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
                        .expect("json! values serialize"),
                    )),
                )?;
            }
            if let Some(note) = convergence_note {
                self.writer.append(
                    EventBuilder::new(EventKind::ContextInject).payload(Payload::Inline(
                        serde_json::to_vec(&serde_json::json!({
                            "what": [note], "why": "convergence",
                        }))
                        .expect("json! values serialize"),
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
                            .expect("json! values serialize"),
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
                            .expect("json! values serialize"),
                        )),
                    )?;
                    Some(msg)
                }
                Some((tool, mut args)) => {
                    // Eric's five #5: the loop injects its own stream id
                    // so a spawned child links to THIS mission. The model
                    // never fabricates delegation provenance.
                    if tool == "agent.spawn" {
                        // Async delegation (Eric ruling 2026-09-08,
                        // D2): the loop mints the child id and books
                        // Spawn BEFORE the call - a crash at any later
                        // point leaves consistent provenance on both
                        // streams (child stream carries child_of).
                        // Pre-flight: without agent.spawn_poll the
                        // outcome could never arrive - fail loudly now.
                        if !self
                            .kernel
                            .list_tools("operator")
                            .iter()
                            .any(|t| t.name == "agent.spawn_poll")
                        {
                            // Mirror the tool-error path exactly: the
                            // model gets its tool result + feedback.
                            // list_tools is subject-filtered: this arm
                            // covers BOTH "not registered" and "registered
                            // but gated away from the operator" - the
                            // diagnosis must say which, or a mis-scoped
                            // config reads as a missing binary.
                            let msg = if self.kernel.has_tool("agent.spawn_poll") {
                                format!(
                                    "tool {tool} failed: agent.spawn_poll is registered but its subjects exclude the operator subject - outcome polls are issued as the operator, so delegation outcomes could never arrive. Fix the config subjects (e.g. subjects = [\"*\"])"
                                )
                            } else {
                                format!(
                                    "tool {tool} failed: agent.spawn requires agent.spawn_poll registered (same hs-plugin-swarm binary) - async delegation polls outcomes through it"
                                )
                            };
                            self.writer.append(
                                EventBuilder::new(EventKind::ToolCall).payload(
                                    Payload::Inline(
                                        serde_json::to_vec(&serde_json::json!({
                                            "plugin": tool, "args": args, "error": msg,
                                        }))
                                        .expect("json! values serialize"),
                                    ),
                                ),
                            )?;
                            pending_feedback.push(format!("harness: {msg}"));
                            continue;
                        }
                        let child_id = uuid::Uuid::new_v4();
                        let cmission =
                            args["mission"].as_str().unwrap_or("").to_string();
                        let cmodel = args["model"]
                            .as_str().map_or_else(|| {
                                self.kernel
                                    .model_names()
                                    .into_iter()
                                    .find(|(_, d)| *d).map_or_else(|| "(unknown)".to_string(), |(n, _)| n)
                            }, std::string::ToString::to_string);
                        self.writer.append(
                            EventBuilder::new(EventKind::Spawn).payload(Payload::Inline(
                                serde_json::to_vec(&serde_json::json!({
                                    "child_stream_id": child_id,
                                    "mission": cmission,
                                    "model": cmodel,
                                }))
                                .expect("json! values serialize"),
                            )),
                        )?;
                        if let Some(sink) = self.ui_sink.as_mut() {
                            sink(uipaint::UiEvent::SubAgentSpawned {
                                child: child_id,
                                parent: Some(self.stream_id),
                                mission: cmission,
                                model: cmodel,
                            });
                        }
                        args["parent_stream"] =
                            serde_json::json!(self.stream_id.to_string());
                        args["child_stream_id"] =
                            serde_json::json!(child_id.to_string());
                        args["depth"] = serde_json::json!(self.swarm_depth);
                    }
                    if let Some(sink) = self.ui_sink.as_mut() {
                        sink(uipaint::UiEvent::ToolCallStart {
                            plugin: tool.clone(),
                            args_summary: uipaint::summarize_args(&args),
                        });
                    }
                    match self.dispatch_tool(&tool, &args) {
                    Ok(tool_out) => {
                        if let Some(sink) = self.ui_sink.as_mut() {
                            let ok = tool_out.output.get("error").is_none_or(serde_json::Value::is_null)
                                && tool_out
                                    .output
                                    .get("exit_code")
                                    .and_then(serde_json::Value::as_i64)
                                    .is_none_or(|c| c == 0);
                            sink(uipaint::UiEvent::ToolCallEnd {
                                plugin: tool.clone(),
                                ok,
                                output_summary: uipaint::summarize_output(&tool_out.output),
                                elapsed_ms: u64::from(tool_out.latency_ms),
                            });
                        }
                        // Async delegation (Eric ruling 2026-09-08):
                        // Spawn was booked BEFORE the call. A running
                        // child registers as pending (its outcome
                        // arrives via poll); a refused/failed spawn
                        // gets its ledger repair + panel finish NOW so
                        // no phantom Running node survives.
                        if tool == "agent.spawn" {
                            let out = &tool_out.output;
                            let failed = out["$error"].is_string()
                                || out["error"].is_string();
                            if let Some(cid) = args["child_stream_id"]
                                .as_str()
                                .and_then(|v| uuid::Uuid::parse_str(v).ok())
                            {
                                if failed {
                                    let _ = self.writer.append(
                                        EventBuilder::new(EventKind::Observation)
                                            .payload(Payload::Inline(
                                                serde_json::to_vec(&serde_json::json!({
                                                    "spawn_failed": cid,
                                                    "detail": out["$error"]
                                                        .as_str()
                                                        .or_else(|| out["error"].as_str())
                                                        .unwrap_or(""),
                                                }))
                                                .expect("json! values serialize"),
                                            )),
                                    );
                                    if let Some(sink) = self.ui_sink.as_mut() {
                                        sink(uipaint::UiEvent::SubAgentFinished {
                                            child: cid,
                                            ok: false,
                                        });
                                    }
                                } else {
                                    self.pending_children.push(PendingChild {
                                        child: cid,
                                        mission: args["mission"]
                                            .as_str()
                                            .unwrap_or("")
                                            .to_string(),
                                        model: out["model"]
                                            .as_str()
                                            .unwrap_or("(unknown)")
                                            .to_string(),
                                    });
                                }
                            }
                        }
                        // D2 (dance #94): book the CANONICAL name the call
                        // actually dispatched to, plus the emitted variant
                        // when it differed - the ledger tells the truth
                        // about both the model's emission and the
                        // resolution (live DeepSeek emitted answer_write /
                        // term.exec / term_exec shapes).
                        let effective = tool_out.resolved.clone().unwrap_or_else(|| tool.clone());
                        let mut rec = serde_json::json!({
                            "plugin": effective, "args": args, "result": tool_out.output,
                        });
                        if effective != tool {
                            rec["requested_as"] = serde_json::json!(tool);
                        }
                        let ev = self.writer.append(
                            EventBuilder::new(EventKind::ToolCall)
                                .payload(Payload::Inline(
                                    serde_json::to_vec(&rec).expect("json! values serialize"),
                                ))
                                .latency_ms(tool_out.latency_ms),
                        )?;
                        // D2: exact duplicate (tool, args) calls get flagged
                        // with the prior seq - an explicit, correctable
                        // signal instead of a silent re-read loop (P3)
                        if let Some(prior) = self.ledger.find_duplicate(&effective, &args) {
                            let note = format!(
                                "duplicate call: identical {tool} args already served at seq {prior} - that result is in your transcript/ledger; do not re-run it"
                            );
                            self.writer.append(
                                EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                                    serde_json::to_vec(&serde_json::json!({
                                        "duplicate_call": tool, "prior_seq": prior, "note": note,
                                    }))
                                    .expect("json! values serialize"),
                                )),
                            )?;
                            pending_feedback.push(note);
                        }
                        self.ledger.apply_tool_call(ev.seq, &effective, &args, &tool_out.output);
                        // Post-B8: guardrail escalation - same-class
                        // edit-path violations are counted per class; from
                        // the second fire on, an escalating steer is
                        // injected (B8's model retried the forbidden class
                        // 6 times against the bare refusal).
                        if effective == "repo.exec"
                            && let Some(class) = repexec::extract_gate_class(&tool_out.output.to_string())
                                && let Some(note) = self.guardrail_escalator.record(&class) {
                                    self.writer.append(
                                        EventBuilder::new(EventKind::ContextInject).payload(
                                            Payload::Inline(
                                                serde_json::to_vec(&serde_json::json!({
                                                    "what": [note.clone()], "why": "guardrail_escalation",
                                                }))
                                                .expect("json! values serialize"),
                                            ),
                                        ),
                                    )?;
                                    pending_feedback.push(note);
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
                                            .expect("json! values serialize"),
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
                                .expect("json! values serialize"),
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
                                    .expect("json! values serialize"),
                                )),
                        )?;
                        // Async delegation: the Spawn was pre-booked -
                        // repair the provenance so no phantom Running
                        // child survives on the stream or the panel.
                        if tool == "agent.spawn"
                            && let Some(cid) = args["child_stream_id"]
                                .as_str()
                                .and_then(|v| uuid::Uuid::parse_str(v).ok())
                            {
                                let _ = self.writer.append(
                                    EventBuilder::new(EventKind::Observation).payload(
                                        Payload::Inline(
                                            serde_json::to_vec(&serde_json::json!({
                                                "spawn_failed": cid, "detail": msg,
                                            }))
                                            .expect("json! values serialize"),
                                        ),
                                    ),
                                );
                                if let Some(sink) = self.ui_sink.as_mut() {
                                    sink(uipaint::UiEvent::SubAgentFinished {
                                        child: cid,
                                        ok: false,
                                    });
                                }
                            }
                        Some(msg)
                    }
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
            // DISC accounting (arXiv 2606.21724 transplant, burn-down 2026-09-09):
            // every red gate verdict is an INTERVENTION; a red carrying
            // reproduced evidence (a failed declared check, a critic
            // refutation) is a DETECTION; infra fail-closed stops count
            // only as interventions - that split is the precision leak
            // the matrix priced.
            if !passed {
                self.idi_interventions += 1;
                if error.contains("declared checks failed")
                    || error.starts_with("critic refuted the submission")
                {
                    self.idi_detections += 1;
                }
            }
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
                            .expect("json! values serialize"),
                        )),
                    )?;
                    if passed && !green {
                        self.writer.append(
                            EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                                serde_json::to_vec(&serde_json::json!({
                                    "conflict": "checker green overrides goal red",
                                    "detail": "goal evaluator vetoed a checker-passed mission - the checker verdict stands (post-A7 rule)",
                                }))
                                .expect("json! values serialize"),
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
                    .expect("json! values serialize"),
                )),
            )?;
            if stop_green {
                // Item 3: adversarial verifier veto (the verifier design).
                // The checker is the ground-truth floor; the verifier runs
                // after green and can only send the work back - never pass
                // on its own authority. Capped rounds; malfunction never blocks.
                const VERIFIER_MAX_ROUNDS: u32 = 3;
                // Feedback integrity F2/F3: how this green resolves is
                // decided per branch and labeled on the result - a capped
                // or malfunction pass is never byte-identical to an
                // audited one.
                let mut outcome = "verified";
                if self.verifier_rounds < VERIFIER_MAX_ROUNDS {
                    self.verifier_rounds += 1;
                    let round = self.verifier_rounds;
                    let answer_text = std::fs::read_to_string(&answer_path).unwrap_or_default();
                    let vprompt = build_verifier_prompt(
                        mission,
                        &answer_text,
                        &self.ledger,
                        &self.prior_gaps,
                    );
                    let verdict_tools = serde_json::json!([crate::toolschema::verdict_tool()]);
                    // M18: the verifier round trip is a real billed model
                    // call - bracket it with UI events like the main path,
                    // or the HUD live count silently drops it (live-proof
                    // capC2: HUD "1 calls" vs done-line "2 calls").
                    if let Some(sink) = self.ui_sink.as_mut() {
                        sink(uipaint::UiEvent::ModelCallStart {
                            model: self.last_model.clone().unwrap_or_default(),
                        });
                    }
                    match self.kernel.call_model_with(
                        "operator",
                        self.model_override.as_deref(),
                        &vprompt,
                        Some(&verdict_tools),
                    ) {
                        Ok(vout) => {
                            model_calls += 1;
                            self.cost_total_micros += vout.cost_usd_micros.max(0) as u64;
                            self.conservative_cost_total_micros += vout.conservative_cost_usd_micros.max(0) as u64;
                            if let Some(sink) = self.ui_sink.as_mut() {
                                sink(uipaint::UiEvent::ModelCallEnd {
                                    model: vout.model.clone(),
                                    input_tokens: vout.input_tokens,
                                    output_tokens: vout.output_tokens,
                                    cost_usd_micros: vout.cost_usd_micros,
                                });
                            }
                                if let Some(ev) = uipaint::reasoning_event(&vout.reasoning_content) {
                                    if let Some(sink) = self.ui_sink.as_mut() {
                                        sink(ev);
                                    }
                                }
                            self.writer.append(
                                EventBuilder::new(EventKind::ModelCall).payload(Payload::Inline(
                                    serde_json::to_vec(&serde_json::json!({
                                        "role": "verifier", "round": round,
                                        "prompt": vprompt, "tools": verdict_tools,
                                        "completion": vout.completion,
                                        "reasoning_tokens": vout.reasoning_tokens,
                                        "reasoning_content": vout.reasoning_content,
                                        "cached_tokens": vout.cached_tokens,
                                        // Accounting reset fix (2026-09-10):
                                        // the in-process counters booked this
                                        // spend but the durable event dropped
                                        // it, so a resume fold under-restored
                                        // (live: C48 41314 of 43600 micros).
                                        "input_tokens": vout.input_tokens,
                                        "output_tokens": vout.output_tokens,
                                        "cost_usd_micros": vout.cost_usd_micros,
                                        "conservative_cost_usd_micros": vout.conservative_cost_usd_micros,
                                    }))
                                    .expect("json! values serialize"),
                                )),
                            )?;
                            // Native verdict (user directive 2026-09-05):
                            // the completion is a verdict.submit tool call;
                            // prose or wrong-tool replies are verifier
                            // errors, never parsed verdicts.
                            let verdict_args =
                                serde_json::from_str::<serde_json::Value>(vout.completion.trim())
                                    .ok()
                                    .filter(|env| env["tool"].as_str() == Some("verdict.submit"))
                                    .map(|env| env["args"].clone());
                            if let Some(v) = verdict_args { match v["refuted"].as_bool() {
                                Some(false) => {
                                    self.prior_gaps.clear();
                                    self.writer.append(
                                        EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                                            serde_json::to_vec(&serde_json::json!({
                                                "why": "verifier", "round": round, "verdict": "not_refuted",
                                            }))
                                            .expect("json! values serialize"),
                                        )),
                                    )?;
                                }
                                Some(true) => {
                                    let findings: Vec<String> = v["findings"]
                                        .as_array()
                                        .map(|a| {
                                            a.iter()
                                                .filter_map(|f| {
                                                    f["detail"].as_str().map(String::from)
                                                })
                                                .collect()
                                        })
                                        .unwrap_or_default();
                                    let blocking =
                                        v["blocking"].as_str().unwrap_or("none").to_string();
                                    self.prior_gaps = findings.clone();
                                    self.writer.append(
                                        EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                                            serde_json::to_vec(&serde_json::json!({
                                                "why": "verifier", "round": round, "verdict": "refuted",
                                                "findings": findings, "blocking": blocking,
                                            }))
                                            .expect("json! values serialize"),
                                        )),
                                    )?;
                                    // DISC accounting: a verifier refutation is a judgment
                                    // intervention WITH findings - detection by construction.
                                    self.idi_interventions += 1;
                                    self.idi_detections += 1;
                                    pending_feedback.push(format!(
                                        "VERIFIER REFUTED (blocking={blocking}): {}",
                                        findings.join("; ")
                                    ));
                                    self.checkpoint(steps, model_calls);
                                    continue;
                                }
                                None => {
                                    outcome = "verifier_malfunction";
                                    self.writer.append(
                                        EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                                            serde_json::to_vec(&serde_json::json!({
                                                "why": "verifier_error", "round": round,
                                                "detail": "verdict JSON missing the refuted field",
                                            }))
                                            .expect("json! values serialize"),
                                        )),
                                    )?;
                                }
                            } } else {
                                outcome = "verifier_malfunction";
                                self.writer.append(
                                    EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                                        serde_json::to_vec(&serde_json::json!({
                                            "why": "verifier_error", "round": round,
                                            "detail": "verdict was not a verdict.submit tool call (prose/wrong-tool reply)",
                                        }))
                                        .expect("json! values serialize"),
                                    )),
                                )?;
                            }
                        }
                        Err(e) => {
                            outcome = "verifier_malfunction";
                            self.writer.append(
                                EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                                    serde_json::to_vec(&serde_json::json!({
                                        "why": "verifier_error", "round": round,
                                        "detail": format!("verifier call failed: {e}"),
                                    }))
                                    .expect("json! values serialize"),
                                )),
                            )?;
                        }
                    }
                } else {
                    outcome = "ratchet_capped";
                    self.writer.append(
                        EventBuilder::new(EventKind::Feedback).payload(Payload::Inline(
                            serde_json::to_vec(&serde_json::json!({
                                "why": "verifier_ratchet", "rounds": VERIFIER_MAX_ROUNDS,
                                "detail": "verifier failed to converge in 3 rounds - the checker verdict stands",
                            }))
                            .expect("json! values serialize"),
                        )),
                    )?;
                }
                // Async delegation: join every running child before
                // the passing close so its cost lands in these books
                // and the panel sees the finish. Bounded by each
                // child's own max_steps; the wall guard is enforced
                // INSIDE the loop - a wedged child must not hang the
                // passing close past the mission's wall.
                while !self.pending_children.is_empty() {
                    let _ = self.poll_children();
                    if self.pending_children.is_empty() {
                        break;
                    }
                    if let Some(res) =
                        self.wall_kill_close(mission, steps, model_calls, &answer_path)?
                    {
                        return Ok(res);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                if self.idi_interventions > 0 {
                    self.idi_repairs += 1;
                }
                self.completion_mode = Some("hybrid");
                self.close_goal(mission, true, outcome)?;
                self.checkpoint(steps, model_calls);
                return Ok(MissionResult {
                    passed: true,
                    steps,
                    model_calls,
                    stream_id: self.stream_id,
                    answer_path,
                    budget_killed: false,
            cost_micros: self.cost_total_micros.saturating_sub(self.mission_cost_start),
            conservative_cost_micros: self
                .conservative_cost_total_micros
                .saturating_sub(self.mission_conservative_start),
                    harness_error: None,
                    outcome: outcome.to_string(),
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
        // Fold whatever children already finished (non-blocking).
        let _ = self.poll_children();
        self.close_goal(mission, false, "steps_exhausted")?;
        Ok(MissionResult {
            passed: false,
            steps,
            model_calls,
            stream_id: self.stream_id,
            answer_path,
            budget_killed: false,
            cost_micros: self.cost_total_micros.saturating_sub(self.mission_cost_start),
            conservative_cost_micros: self
                .conservative_cost_total_micros
                .saturating_sub(self.mission_conservative_start),
            harness_error: None,
            outcome: "steps_exhausted".to_string(),
        })
    }
}

/// Book a wall-clock kill from a loop checkpoint (phase 1, T5): the run
/// did real work up to `steps`, so the result row must carry it - the old
/// runner wrote steps:0 on timeout, which both hid progress and poisoned
/// per-step cost accounting.
#[must_use]
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

/// Item 3: the verifier's prompt. Audit-recorded-evidence only;
/// default-refuted on uncertainty; anti-ratchet on re-rounds
/// (the verifier design; Grok `goal_verifier_prompt.md` adapted).
fn build_verifier_prompt(
    mission: &str,
    answer: &str,
    ledger: &crate::ledger::Ledger,
    prior_gaps: &[String],
) -> String {
    let gaps = if prior_gaps.is_empty() {
        "none".to_string()
    } else {
        prior_gaps
            .iter()
            .map(|g| format!("- {g}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut p = String::new();
    p.push_str("ADVERSARIAL VERIFIER\n");
    p.push_str("You are not the agent that did this work. Default to refuted when uncertain a required criterion holds; never invent requirements. Audit the RECORDED evidence only - a prose claim of test output with no recorded run is fabricated: refute. On a re-verification round (PRIOR_GAPS non-empty), check that each prior gap is genuinely fixed plus demonstrable defects; a fresh stylistic objection a prior round implicitly accepted is out of scope - when every prior gap is fixed and the objective holds, return refuted false.\n");
    p.push_str(&format!("OBJECTIVE: {mission}\n"));
    p.push_str(&format!("ANSWER:\n{answer}\n"));
    p.push_str(&format!(
        "LEDGER (recorded evidence):\n{}\n",
        ledger.summary()
    ));
    p.push_str(&format!("PRIOR_GAPS:\n{gaps}\n"));
    p.push_str("Submit the verdict by calling the verdict.submit tool exactly once - never prose, never bare JSON.");

    p
}

/// The kernel constructor every production SWE runner path must use
/// (hs-swe-run, swarm children). 2026-09-07 wiring gap: hs-swe-run built its
/// kernel with `Kernel::load` (no log root), which silently disabled the
/// af5f7b57 wedge-visibility records and stderr capture on the live path
/// while the RED test proved them under `load_with_log`. One constructor keeps
/// the log root non-optional on the run path.
pub fn swe_kernel(
    config: &std::path::Path,
    log_root: &std::path::Path,
) -> Result<hs_kernel::Kernel, hs_kernel::KernelError> {
    hs_kernel::Kernel::load_with_log(config, log_root)
}

/// Startup gate for every production runner: refuse to run blind.
/// 2026-09-07: hs-swe-run ran the 17302 audit with a log-root-less kernel
/// and emitted zero dispatch records - the wedge-visibility fix was compiled
/// in but dead. A kernel without a log root must stop the run, not silently
/// disable observability.
pub fn require_visibility(k: &hs_kernel::Kernel) -> Result<(), String> {
    if k.has_log_root() {
        Ok(())
    } else {
        Err("kernel has no log root: visibility wiring inactive (dispatch records, stderr capture dead) - refusing to run blind".into())
    }
}

/// Render the volatile ARTIFACT block (octodns-1298, 2026-09-07): this is
/// the answer FILE as it stands on disk - the exact bytes the checker
/// grades - updated ONLY by answer.submit. It is NOT live candidate state;
/// the old bare "ARTIFACT:" label led the model to read it as the current
/// candidate and loop on phantom stale versions after edit.patch reset +
/// re-apply. The label now says what it is and what writes it. Content is
/// shown untrimmed except for the trailing newline run: structural
/// whitespace (a blank context line) is load-bearing in diffs.
#[must_use]
pub fn artifact_section(answer_path: &std::path::Path, artifact: &str) -> String {
    let shown = if artifact.is_empty() {
        "<none>".to_string()
    } else {
        artifact.trim_end_matches('\n').to_string()
    };
    format!(
        "\nANSWER_PATH: {}\nARTIFACT (the graded answer file on disk - updated ONLY by answer.submit; NOT live candidate state):\n{}\n",
        answer_path.display(),
        shown
    )
}

/// Default per-mission step cap for the live REPL loop (`hs-repl
/// --max-steps`). Measured 2026-09-10 (cap matrix, P48): 25 exhausts a
/// refute-history mission before the corrected submit can verdict.
pub const DEFAULT_MISSION_MAX_STEPS: u32 = 50;
