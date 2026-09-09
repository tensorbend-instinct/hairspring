//! HAIRSPRING gate 8, self-modification half (spec figure 5, right column).
//!
//! The agent's mutable surface is the POLICY LAYER: prompt text and tool
//! configuration. The loop is:
//!
//!   fork -> quarantine -> apply mutation -> pin scorer -> assay (held-out)
//!   -> soak -> promote (lineage + `capability_delta/fitness_delta` events)
//!   or rewind to known-good.
//!
//! Side-effect discipline: a fork runs under world-service quarantine; any
//! external effect (send / spend / write outside the sandbox) is rejected by
//! the world service until the fork's lineage is promoted. Proof tests in
//! `tests/gate8_selfmod_proof.rs` pin every one of these properties.

pub mod migration;

use hs_core::{Event, EventBuilder, EventKind, Payload};
use hs_log::{StreamReader, StreamWriter};
use hs_scorer::{
    Artifact, AssayVerdict, Candidate, Lineage, PromotionError, Scorer, ScorerPin, Task, TaskSuite,
    Tier01, Tier02,
};
pub mod cycle;

pub use hs_world::Effect;
use hs_world::{World, WorldError};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use uuid::Uuid;

// --------------------------------------------------------- policy layer ---

/// A tool configuration the agent may rewrite. The gate-8 deterministic
/// world interprets the "answer" tool: `PrefixRule` derives secrets by rule
/// (generalizes), Table is a memorization table (does not).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyTool {
    PrefixRule,
    Table(BTreeMap<String, String>),
}

/// The mutable surface of the agent: prompts + tool configs. Mutations may
/// touch ONLY this layer (spec fig 5: "rewrite a prompt or a tool").
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyLayer {
    pub prompts: BTreeMap<String, String>,
    pub tools: BTreeMap<String, PolicyTool>,
}
impl PolicyLayer {
    #[must_use]
    pub fn new(prompts: BTreeMap<String, String>, tools: BTreeMap<String, PolicyTool>) -> Self {
        PolicyLayer { prompts, tools }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PolicyChange {
    SetPrompt { name: String, text: String },
    SetTool { name: String, tool: PolicyTool },
}

/// A proposed self-modification. By construction it can only express
/// policy-layer changes; there is no variant that touches code, the log, or
/// the scorer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mutation {
    changes: Vec<PolicyChange>,
}
impl Mutation {
    #[must_use]
    pub fn new(changes: Vec<PolicyChange>) -> Self {
        Mutation { changes }
    }
    #[must_use]
    pub fn changes(&self) -> &[PolicyChange] {
        &self.changes
    }
}

// ------------------------------------------------------------------ fork ---

/// A quarantined copy of the live policy layer. Forks are short-lived and
/// in-process; the world service holds the quarantine mark on the fork's
/// stream for the fork's whole life.
pub struct Fork {
    stream: Uuid,
    policy: PolicyLayer,
    candidate: String,
}
impl Fork {
    #[must_use]
    pub fn stream(&self) -> Uuid {
        self.stream
    }
    #[must_use]
    pub fn candidate_name(&self) -> &str {
        &self.candidate
    }
    #[must_use]
    pub fn policy(&self) -> &PolicyLayer {
        &self.policy
    }
    /// Interpret the policy layer as a scorable artifact (token family).
    #[must_use]
    pub fn candidate(&self) -> Candidate {
        let artifact = match self.policy.tools.get("answer") {
            Some(PolicyTool::PrefixRule) => Artifact::by_rule(|t: &Task| {
                let i: usize = t.id[1..].parse().ok()?;
                Some(format!(
                    "{}-{i}",
                    if t.id.starts_with('V') {
                        "VISIBLE"
                    } else {
                        "HIDDEN"
                    }
                ))
            }),
            Some(PolicyTool::Table(m)) => Artifact::memorized(m.clone()),
            None => Artifact::memorized(BTreeMap::new()),
        };
        Candidate::new(&self.candidate, artifact)
    }
}

// ---------------------------------------------------------------- errors ---

#[derive(Debug)]
pub enum SelfModError {
    /// The world service rejected an external effect from quarantine.
    QuarantinedEffect(String),
    /// Promotion attempted before the soak window elapsed.
    SoakNotElapsed {
        remaining_ms: u64,
    },
    /// Promotion attempted with no assay on record for this fork.
    NoAssay,
    Promotion(String),
    /// v5 per-cycle balance rule violated (refused before the fork exists).
    UnbalancedCycle(String),
    /// `closes_failure` names no open failure/regression in evidence state.
    EvidenceMismatch(String),
    /// v5 frozen-candidate rule: the verdict does not name this fork's
    /// candidate - "the producer's account" never promotes.
    FrozenMismatch(String),
    World(WorldError),
    Log(hs_log::LogError),
    Io(std::io::Error),
}
impl From<WorldError> for SelfModError {
    fn from(e: WorldError) -> Self {
        SelfModError::World(e)
    }
}
impl From<hs_log::LogError> for SelfModError {
    fn from(e: hs_log::LogError) -> Self {
        SelfModError::Log(e)
    }
}
impl From<PromotionError> for SelfModError {
    fn from(e: PromotionError) -> Self {
        SelfModError::Promotion(format!("{e:?}"))
    }
}
impl From<std::io::Error> for SelfModError {
    fn from(e: std::io::Error) -> Self {
        SelfModError::Io(e)
    }
}

// ------------------------------------------------------------ the loop ---

pub struct SelfModLoop {
    world: World,
    scorer: Scorer,
    lineage: Lineage,
    current: PolicyLayer,
    soak: Duration,
    stream: Uuid,
    writer: StreamWriter,
    log_root: PathBuf,
    assays: HashMap<Uuid, Instant>,
}

impl SelfModLoop {
    pub fn new(
        world: World,
        scorer: Scorer,
        lineage: Lineage,
        seed: PolicyLayer,
        soak: Duration,
    ) -> Self {
        let log_root = world.log_root().to_path_buf();
        let stream = Uuid::new_v4();
        let writer = StreamWriter::create(&log_root, stream).expect("selfmod stream");
        SelfModLoop {
            world,
            scorer,
            lineage,
            current: seed,
            soak,
            stream,
            writer,
            log_root,
            assays: HashMap::new(),
        }
    }

    pub fn current_policy(&self) -> &PolicyLayer {
        &self.current
    }

    fn emit(&mut self, kind: EventKind, body: &str) -> Result<(), SelfModError> {
        let hash = hs_log::write_blob(&self.log_root, body.as_bytes())?;
        self.writer
            .append(EventBuilder::new(kind).payload(Payload::BlobRef {
                hash,
                len: body.len() as u64,
            }))?;
        Ok(())
    }

    /// Evidence state read-through for the proposer (spec v5: the
    /// proposer consumes evidence state, not raw artifact state).
    pub fn evidence_state(&self) -> Vec<hs_scorer::evidence::EvidenceClaim> {
        self.scorer.evidence_state()
    }

    pub fn scorer_mut(&mut self) -> &mut Scorer {
        &mut self.scorer
    }

    /// v5 per-cycle balance: fork only a balanced cycle - one open failure
    /// closed AND one bounded capability added, both validated against the
    /// current evidence state before any fork exists.
    pub fn fork_cycle(&mut self, proposal: &cycle::CycleProposal) -> Result<Fork, SelfModError> {
        cycle::check_balance(proposal, &self.scorer.evidence_state())?;
        Ok(self.fork())
    }

    /// Fork the live policy layer into a quarantined assay fork.
    pub fn fork(&mut self) -> Fork {
        let stream = Uuid::new_v4();
        self.world.quarantine(stream);
        Fork {
            stream,
            policy: self.current.clone(),
            candidate: format!("fork-{}", &stream.simple().to_string()[..8]),
        }
    }

    /// Pin scorer version + assay conditions BEFORE the mutation is
    /// evaluated: the mutation can never move the goalposts (spec fig 5).
    pub fn pin_scorer(&mut self) -> ScorerPin {
        self.scorer.pin()
    }

    /// Apply a mutation to a fork's policy layer; recorded as a Mutation
    /// substrate event.
    pub fn apply(&mut self, fork: &mut Fork, m: Mutation) -> Result<(), SelfModError> {
        for ch in &m.changes {
            match ch {
                PolicyChange::SetPrompt { name, text } => {
                    fork.policy.prompts.insert(name.clone(), text.clone());
                }
                PolicyChange::SetTool { name, tool } => {
                    fork.policy.tools.insert(name.clone(), tool.clone());
                }
            }
        }
        let body = serde_json::to_string(&serde_json::json!({
            "fork": fork.stream.to_string(),
            "changes": m.changes(),
        }))
        .expect("json! values serialize");
        self.emit(EventKind::Mutation, &body)?;
        Ok(())
    }

    /// Tier 0-1 self-report on the visible suite (what the agent can see).
    pub fn run_visible(&mut self, fork: &Fork, suite: &TaskSuite) -> Result<Tier01, SelfModError> {
        Ok(self.scorer.tier01_execution(&fork.candidate(), suite)?)
    }

    /// Held-out assay under a frozen pin. This is the promotion gate; the
    /// soak window starts when the assay lands.
    pub fn assay(
        &mut self,
        fork: &Fork,
        heldout: &TaskSuite,
        pin: &ScorerPin,
    ) -> Result<AssayVerdict, SelfModError> {
        let v = self
            .scorer
            .held_out_assay(&fork.candidate(), heldout, pin)?;
        self.assays.insert(fork.stream, Instant::now());
        Ok(v)
    }

    /// Promote a fork after the soak: lineage records the held-out scores +
    /// pin, the mutated policy becomes live, quarantine lifts, and
    /// `capability_delta` + `fitness_delta` events land on the canonical log.
    pub fn promote(
        &mut self,
        fork: &Fork,
        verdict: &AssayVerdict,
        pin: &ScorerPin,
    ) -> Result<(), SelfModError> {
        // independent acceptance on the FROZEN candidate (spec v5): the
        // verdict must be about the candidate exactly as produced - this
        // fork's own - never the producer's account of another candidate
        if verdict.candidate != fork.candidate_name() {
            return Err(SelfModError::FrozenMismatch(format!(
                "verdict names '{}', fork candidate is '{}'",
                verdict.candidate,
                fork.candidate_name()
            )));
        }
        let at = self.assays.get(&fork.stream).ok_or(SelfModError::NoAssay)?;
        let elapsed = at.elapsed();
        if elapsed < self.soak {
            return Err(SelfModError::SoakNotElapsed {
                remaining_ms: self.soak.checked_sub(elapsed).unwrap().as_millis() as u64,
            });
        }
        let cand = fork.candidate();
        let s0 = Tier01 {
            passed: verdict.passed(),
            tasks_correct: verdict.tasks_passed,
            tasks_total: verdict.tasks_total,
        };
        // The held-out assay IS the deciding tier; tier-2 fields mirror the
        // objective rate (no rubric judges in the gate-8 deterministic world).
        let s1 = Tier02 {
            mean: verdict.pass_rate,
            ci_low: verdict.pass_rate,
            ci_high: verdict.pass_rate,
            veto: !verdict.passed(),
            cross_family_disagreement: 0.0,
            same_family_disagreement: 0.0,
        };
        self.lineage.record(&cand, s0, s1, verdict.clone());
        self.lineage
            .promote(&cand, verdict, pin, &self.scorer)
            .map_err(|e| SelfModError::Promotion(format!("{e:?}")))?;

        self.current = fork.policy.clone();
        self.world.lift_quarantine(fork.stream);
        self.emit(
            EventKind::CapabilityDelta,
            &format!(
                "capability_delta candidate={} promoted=true prompts={} tools={}",
                cand.name(),
                self.current.prompts.len(),
                self.current.tools.len()
            ),
        )?;
        self.emit(
            EventKind::FitnessDelta,
            &format!(
                "fitness_delta candidate={} held_out_pass_rate={:.3}",
                cand.name(),
                verdict.pass_rate
            ),
        )?;
        Ok(())
    }

    /// Rewind: drop the fork, lift its quarantine, leave the live policy
    /// layer exactly as it was (known-good). The mutation never touched it.
    pub fn rewind(&mut self, fork: Fork) -> Result<(), SelfModError> {
        self.world.lift_quarantine(fork.stream);
        self.assays.remove(&fork.stream);
        self.emit(
            EventKind::Mutation,
            &format!("rewind fork={} restored known-good policy", fork.stream),
        )?;
        Ok(())
    }

    /// Route an effect attempt through the world service (the authority).
    pub fn attempt_effect(&self, fork: &Fork, effect: Effect) -> Result<(), SelfModError> {
        self.world
            .authorize_effect(fork.stream, effect)
            .map_err(|e| match e {
                WorldError::Rejected(m) => SelfModError::QuarantinedEffect(m),
                other => SelfModError::World(other),
            })
    }

    /// Self-mod stream events (Mutation / `CapabilityDelta` / `FitnessDelta`).
    pub fn selfmod_events(&self) -> Vec<Event> {
        StreamReader::open(&self.log_root, self.stream)
            .and_then(|r| r.events())
            .unwrap_or_default()
    }

    /// Scorer stream events (Score / `ScorerPin` / Regression / `CanaryResult`).
    pub fn scorer_events(&self) -> Vec<Event> {
        self.scorer.log_events()
    }
}
