//! HAIRSPRING gate 7 (spec section 10, row 7): the scorer f - tiered
//! pipeline (0-1 objective, 2 rubric ensemble under hygiene, 3 held-out
//! assay), scorer pinning, canary suite, and the best-of-N baseline.
//!
//! Safety-case invariants enforced here (spec "The safety case"):
//! - champion status is decided by the held-out tier ONLY (cheap gates run
//!   first; no candidate becomes champion on self-adjacent evidence);
//! - scorer version + assay conditions are pinned BEFORE mutation and every
//!   promotion is bound to that pin (`scorer_pin` event; replayable verdicts);
//! - canary outcomes are substrate events (`canary_result`); a scorer whose
//!   canary error rises is frozen for promotion decisions until re-anchored;
//! - the best-of-N envelope is endpoint-wise with identical decision
//!   opportunities, and the comparison is published win or lose.

pub mod attribution;
pub mod evidence;

use evidence::{ClaimKind, EvidenceClaim};
use hs_core::{Event, EventBuilder, EventKind, Payload};
use hs_log::{StreamReader, StreamWriter};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use uuid::Uuid;

// ---------------------------------------------------------------- tasks ---

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    pub id: String,
    expected: String,
}
impl Task {
    #[must_use]
    pub fn new(id: String, secret: String) -> Self {
        Task {
            id,
            expected: secret,
        }
    }
    #[must_use]
    pub fn secret(&self) -> &str {
        &self.expected
    }
}

#[derive(Clone, Debug)]
pub struct TaskSuite {
    name: String,
    tasks: Vec<Task>,
}
impl TaskSuite {
    #[must_use]
    pub fn new(name: &str, tasks: Vec<Task>) -> Self {
        TaskSuite {
            name: name.into(),
            tasks,
        }
    }
    #[must_use]
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

// ------------------------------------------------------------ artifacts ---

type AnswerRule = Rc<dyn Fn(&Task) -> Option<String>>;

enum Answerer {
    Rule(AnswerRule),
    Table(BTreeMap<String, String>),
}

/// A candidate's produced artifact: either a rule (generalizes) or a
/// memorized table (passes only what it has seen).
#[derive(Clone)]
pub struct Artifact {
    answerer: Rc<Answerer>,
    current: Rc<RefCell<Option<String>>>,
}
impl Artifact {
    pub fn by_rule<F: Fn(&Task) -> Option<String> + 'static>(f: F) -> Self {
        Artifact {
            answerer: Rc::new(Answerer::Rule(Rc::new(f))),
            current: Rc::new(RefCell::new(None)),
        }
    }
    #[must_use]
    pub fn memorized(table: BTreeMap<String, String>) -> Self {
        Artifact {
            answerer: Rc::new(Answerer::Table(table)),
            current: Rc::new(RefCell::new(None)),
        }
    }
    /// Answer a task; also recorded as the artifact's "current" answer so
    /// rubric judges see exactly what the execution tier produced.
    pub fn answer(&self, task: &Task) -> Option<String> {
        let a = match &*self.answerer {
            Answerer::Rule(f) => f(task),
            Answerer::Table(t) => t.get(&task.id).cloned(),
        };
        *self.current.borrow_mut() = a.clone();
        a
    }
    #[must_use]
    pub fn answer_text(&self) -> Option<String> {
        self.current.borrow().clone()
    }
}

#[derive(Clone)]
pub struct Candidate {
    name: String,
    artifact: Artifact,
}
impl Candidate {
    #[must_use]
    pub fn new(name: &str, artifact: Artifact) -> Self {
        Candidate {
            name: name.into(),
            artifact,
        }
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn artifact(&self) -> &Artifact {
        &self.artifact
    }
}

// --------------------------------------------------------------- judges ---

pub trait Judge {
    fn name(&self) -> &str;
    fn family(&self) -> &str;
    fn score(&self, art: &Artifact, task: &Task) -> f64;
}

pub struct ClosureJudge<F: Fn(&Artifact, &Task) -> f64> {
    name: String,
    family: String,
    f: F,
}
impl<F: Fn(&Artifact, &Task) -> f64> ClosureJudge<F> {
    pub fn new(name: &str, family: &str, f: F) -> Self {
        ClosureJudge {
            name: name.into(),
            family: family.into(),
            f,
        }
    }
}
impl<F: Fn(&Artifact, &Task) -> f64> Judge for ClosureJudge<F> {
    fn name(&self) -> &str {
        &self.name
    }
    fn family(&self) -> &str {
        &self.family
    }
    fn score(&self, art: &Artifact, task: &Task) -> f64 {
        (self.f)(art, task)
    }
}

pub struct JudgePanel {
    judges: Vec<Box<dyn Judge>>,
}
impl JudgePanel {
    #[must_use]
    pub fn new(judges: Vec<Box<dyn Judge>>) -> Self {
        JudgePanel { judges }
    }
    #[must_use]
    pub fn judge_names(&self) -> Vec<String> {
        self.judges.iter().map(|j| j.name().to_string()).collect()
    }
}

// --------------------------------------------------------------- scores ---

#[derive(Clone, Debug, PartialEq)]
pub struct Tier01 {
    pub passed: bool,
    pub tasks_correct: u32,
    pub tasks_total: u32,
}

/// Tier-2 rubric ensemble output: a distribution, not a point (spec: CI,
/// never sole gate), with cross-/same-family disagreement tracked separately.
#[derive(Clone, Debug, PartialEq)]
pub struct Tier02 {
    pub mean: f64,
    pub ci_low: f64,
    pub ci_high: f64,
    pub veto: bool,
    /// Distinct judge families on the panel (cross-family hygiene).
    pub families: usize,
    /// Spec tier-2 hygiene: where only one model family is available,
    /// the veto weight is reduced (and the record says so).
    pub veto_weight: f64,
    pub cross_family_disagreement: f64,
    pub same_family_disagreement: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssayVerdict {
    pub candidate: String,
    pub suite: String,
    pub pass_rate: f64,
    pub tasks_passed: u32,
    pub tasks_total: u32,
    pin_hash: [u8; 32],
}
impl AssayVerdict {
    #[must_use]
    pub fn pin_hash(&self) -> [u8; 32] {
        self.pin_hash
    }
    #[must_use]
    pub fn passed(&self) -> bool {
        self.pass_rate == 1.0
    }
}

// ----------------------------------------------------------------- pins ---

/// Scorer version + assay conditions, frozen BEFORE mutation (spec: a
/// mutation cannot move its own goalposts; recorded as a `scorer_pin` event).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScorerPin {
    scorer_version: String,
    conditions: String,
    hash: [u8; 32],
}
impl ScorerPin {
    #[must_use]
    pub fn hash(&self) -> [u8; 32] {
        self.hash
    }
    fn compute(scorer_version: &str, conditions: &str) -> [u8; 32] {
        // Pure computation: sha256 over "version|conditions". NEVER a
        // storage call - the previous write_blob(/tmp) route littered the
        // blob store and, on an IO error, silently returned a zero hash
        // that would make any two pins "match".
        use sha2::Digest;
        sha2::Sha256::digest(format!("{scorer_version}|{conditions}").as_bytes()).into()
    }
}

// -------------------------------------------------------------- canaries ---

pub struct Canary {
    id: String,
    artifact: Artifact,
    ground_truth_good: bool,
}
impl Canary {
    #[must_use]
    pub fn known_good(id: &str, artifact: Artifact) -> Self {
        Canary {
            id: id.into(),
            artifact,
            ground_truth_good: true,
        }
    }
    #[must_use]
    pub fn known_bad(id: &str, artifact: Artifact) -> Self {
        Canary {
            id: id.into(),
            artifact,
            ground_truth_good: false,
        }
    }
}

pub struct CanarySuite {
    canaries: Vec<Canary>,
}
impl CanarySuite {
    #[must_use]
    pub fn new(canaries: Vec<Canary>) -> Self {
        CanarySuite { canaries }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CanaryReport {
    pub error_rate: f64,
    pub drifted: bool,
    pub results: Vec<(String, bool, bool)>, // (id, ground_truth_good, scorer_said_good)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriftKind {
    /// The held-out evaluator reads every artifact as passing.
    HeldOutAlwaysPasses,
}

// -------------------------------------------------------------- lineage ---

#[derive(Debug, Clone, PartialEq)]
pub enum PromotionError {
    AssayFailed { pass_rate: f64 },
    PinMismatch { expected: [u8; 32], found: [u8; 32] },
    ScorerFrozen,
    NoVerdict,
}
impl std::fmt::Display for PromotionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PromotionError::AssayFailed { pass_rate } => {
                write!(f, "held-out assay failed (pass_rate={pass_rate})")
            }
            PromotionError::PinMismatch { .. } => {
                write!(f, "scorer pin mismatch - goalposts moved after pin")
            }
            PromotionError::ScorerFrozen => write!(f, "scorer frozen by canary drift"),
            PromotionError::NoVerdict => write!(f, "no held-out verdict recorded for candidate"),
        }
    }
}
impl std::error::Error for PromotionError {}

struct Entry {
    candidate: Candidate,
    // tier scores retained as the durable record of why the verdict
    // landed; the in-memory reads go through `verdict` (champion selection
    // is held-out only), so these fields are write-only by design
    _s0: Tier01,
    _s1: Tier02,
    verdict: AssayVerdict,
}

/// The lineage: every candidate with its tier scores; champion = best by
/// held-out only.
pub struct Lineage {
    dir: PathBuf,
    family: String,
    entries: Vec<Entry>,
    champion: Option<String>,
}
impl Lineage {
    pub fn new(dir: PathBuf, family: &str) -> Result<Self, std::io::Error> {
        std::fs::create_dir_all(&dir)?;
        Ok(Lineage {
            dir,
            family: family.into(),
            entries: vec![],
            champion: None,
        })
    }
    pub fn record(&mut self, candidate: &Candidate, s0: Tier01, s1: Tier02, verdict: AssayVerdict) {
        self.entries.push(Entry {
            candidate: candidate.clone(),
            _s0: s0,
            _s1: s1,
            verdict,
        });
    }
    #[must_use]
    pub fn champion(&self) -> Option<&Candidate> {
        let name = self.champion.as_ref()?;
        self.entries
            .iter()
            .find(|e| &e.candidate.name == name)
            .map(|e| &e.candidate)
    }
    /// The only promotion gate: held-out verdict under the frozen pin, with
    /// the scorer not drift-frozen. Tier 0-1 and tier 2 never gate champion
    /// status (spec: only s2 gates champion status).
    pub fn promote(
        &mut self,
        candidate: &Candidate,
        verdict: &AssayVerdict,
        pin: &ScorerPin,
        scorer: &Scorer,
    ) -> Result<(), PromotionError> {
        if scorer.is_frozen() {
            return Err(PromotionError::ScorerFrozen);
        }
        scorer.verify_pin(verdict, pin)?;
        if !verdict.passed() {
            return Err(PromotionError::AssayFailed {
                pass_rate: verdict.pass_rate,
            });
        }
        let beats = match &self.champion {
            None => true,
            Some(name) => {
                let champ_rate = self
                    .entries
                    .iter()
                    .find(|e| &e.candidate.name == name)
                    .map_or(0.0, |e| e.verdict.pass_rate);
                verdict.pass_rate >= champ_rate
            }
        };
        if beats {
            self.champion = Some(candidate.name().to_string());
            // lineage record is a durable substrate artifact
            let rec = self
                .dir
                .join(format!("promotion-{}.json", candidate.name()));
            // Real JSON (never format!-rolled): names may contain quotes.
            let body = serde_json::json!({
                "family": self.family,
                "candidate": candidate.name(),
                "held_out_pass_rate": verdict.pass_rate,
                "pin": hex(pin.hash()),
            })
            .to_string();
            let _ = std::fs::write(rec, body);
        }
        Ok(())
    }
}

fn hex(b: [u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// --------------------------------------------------------------- scorer ---

#[derive(Clone, Debug)]
pub struct ScorerConfig {
    /// Canary error rate above which the scorer freezes for promotion.
    pub canary_error_threshold: f64,
    /// Mean absolute deviation from human-labeled anchors above which a
    /// judge is dropped from the panel (re-anchored or dropped).
    pub anchor_drift_threshold: f64,
    pub scorer_version: String,
    pub assay_conditions: String,
}
impl Default for ScorerConfig {
    fn default() -> Self {
        ScorerConfig {
            canary_error_threshold: 0.0,
            anchor_drift_threshold: 0.25,
            scorer_version: "hs-scorer-0.1.0".into(),
            assay_conditions: "token-family-heldout-v1".into(),
        }
    }
}

pub struct Scorer {
    writer: StreamWriter,
    log_root: PathBuf,
    stream: Uuid,
    config: ScorerConfig,
    drift: Option<DriftKind>,
    frozen: bool,
    pinned: Option<ScorerPin>,
}
impl Scorer {
    pub fn new(log_root: &Path, config: ScorerConfig) -> Result<Self, hs_log::LogError> {
        std::fs::create_dir_all(log_root).map_err(hs_log::LogError::Io)?;
        let stream = Uuid::new_v4();
        let writer = StreamWriter::create(log_root, stream)?;
        hs_log::register_stream(log_root, "scorer", stream)?;
        Ok(Scorer {
            writer,
            log_root: log_root.to_path_buf(),
            stream,
            config,
            drift: None,
            frozen: false,
            pinned: None,
        })
    }

    /// This scorer's canonical stream. Registered under the `scorer`
    /// role at creation (B8 discovery for off-process read surfaces).
    #[must_use]
    pub fn stream(&self) -> Uuid {
        self.stream
    }

    fn emit(&mut self, kind: EventKind, body: &str) -> Event {
        // Fail-stop by design: the canonical log is the scorer's evidence
        // base, and continuing past a failed write would let a verdict rest
        // on silently incomplete evidence. Abort loudly instead.
        let hash = hs_log::write_blob(&self.log_root, body.as_bytes())
            .expect("canonical log write must succeed: partial evidence is worse than a halt");
        self.writer
            .append(EventBuilder::new(kind).payload(Payload::BlobRef {
                hash,
                len: body.len() as u64,
            }))
            .expect("canonical log append must succeed: partial evidence is worse than a halt")
    }

    /// GATE 9c (spec v5): evidence state, projected from the canonical log.
    /// "An artifact says what exists; evidence says what is known about it."
    /// The proposer in the evolutionary loop consumes THIS, not raw
    /// artifacts: what is verified, what is failing, what regressed.
    #[must_use]
    pub fn evidence_state(&self) -> Vec<EvidenceClaim> {
        let events = self.log_events();
        let reader = StreamReader::open(&self.log_root, self.stream)
            .expect("reading the canonical log this scorer wrote");
        evidence::project(&events, &|e| {
            reader
                .resolve_payload(e)
                .ok()
                .map(|b| String::from_utf8_lossy(&b).into_owned())
        })
    }

    #[must_use]
    pub fn open_failures(&self) -> Vec<EvidenceClaim> {
        self.evidence_state()
            .into_iter()
            .filter(|c| c.kind == ClaimKind::OpenFailure && c.status == evidence::ClaimStatus::Open)
            .collect()
    }

    #[must_use]
    pub fn regressions(&self) -> Vec<EvidenceClaim> {
        self.evidence_state()
            .into_iter()
            .filter(|c| c.kind == ClaimKind::Regression && c.status == evidence::ClaimStatus::Open)
            .collect()
    }

    #[must_use]
    pub fn verified_claims(&self) -> Vec<EvidenceClaim> {
        self.evidence_state()
            .into_iter()
            .filter(|c| {
                c.kind == ClaimKind::VerifiedClaim && c.status == evidence::ClaimStatus::Open
            })
            .collect()
    }

    /// GATE 9e (spec v5): record a capability swap as a first-class
    /// `capability_change` event on this stream. Single-writer discipline:
    /// whoever performs the swap (migration transaction, operator) records
    /// it BEFORE the next assay so attribution sees the boundary.
    pub fn record_capability_change(&mut self, binding: &str, reason: &str) -> Event {
        self.emit(
            EventKind::CapabilityChange,
            &format!("capability_change binding={binding} reason={reason}"),
        )
    }

    /// GATE 9e (spec v5): fitness slope, projected from the canonical log.
    /// Only deltas with NO `capability_change` between the two assays -
    /// same substrate, same bindings, evolved policy.
    #[must_use]
    pub fn fitness_deltas(&self) -> Vec<attribution::FitnessDeltaRec> {
        let events = self.log_events();
        let reader = StreamReader::open(&self.log_root, self.stream)
            .expect("reading the canonical log this scorer wrote");
        attribution::project(&events, &|e| {
            reader
                .resolve_payload(e)
                .ok()
                .map(|b| String::from_utf8_lossy(&b).into_owned())
        })
        .0
    }

    /// GATE 9e (spec v5): deltas straddling a `capability_change` boundary,
    /// attributed to the swap and recorded against the new binding. Never
    /// counted as evolved improvement.
    #[must_use]
    pub fn capability_attributed(&self) -> Vec<attribution::AttributedDelta> {
        let events = self.log_events();
        let reader = StreamReader::open(&self.log_root, self.stream)
            .expect("reading the canonical log this scorer wrote");
        attribution::project(&events, &|e| {
            reader
                .resolve_payload(e)
                .ok()
                .map(|b| String::from_utf8_lossy(&b).into_owned())
        })
        .1
    }

    /// Pin scorer version + assay conditions BEFORE any mutation; recorded
    /// as a `scorer_pin` substrate event.
    pub fn pin(&mut self) -> ScorerPin {
        self.pin_with_conditions(&self.config.assay_conditions.clone())
    }
    pub fn pin_with_conditions(&mut self, conditions: &str) -> ScorerPin {
        let pin = ScorerPin {
            scorer_version: self.config.scorer_version.clone(),
            conditions: conditions.into(),
            hash: ScorerPin::compute(&self.config.scorer_version, conditions),
        };
        self.emit(
            EventKind::ScorerPin,
            &format!(
                "scorer_pin version={} conditions={} hash={}",
                pin.scorer_version,
                pin.conditions,
                hex(pin.hash)
            ),
        );
        self.pinned = Some(pin.clone());
        pin
    }

    /// Tier 0-1: objective execution on the (visible) suite.
    pub fn tier01_execution(
        &mut self,
        cand: &Candidate,
        suite: &TaskSuite,
    ) -> Result<Tier01, std::io::Error> {
        let mut correct = 0u32;
        for t in suite.tasks() {
            if cand.artifact().answer(t).as_deref() == Some(t.secret()) {
                correct += 1;
            }
        }
        let r = Tier01 {
            passed: correct as usize == suite.tasks().len(),
            tasks_correct: correct,
            tasks_total: suite.tasks().len() as u32,
        };
        // regression bookkeeping (spec v5): snapshot the prior claim BEFORE
        // this score lands, so a verified subject failing is tracked, never
        // overwritten
        let prior = self
            .evidence_state()
            .into_iter()
            .find(|c| c.subject == cand.name() && c.status == evidence::ClaimStatus::Open);
        let score_ev = self.emit(
            EventKind::Score,
            &format!(
                "tier01 candidate={} suite={} passed={} {}/{}",
                cand.name(),
                suite.name(),
                r.passed,
                correct,
                r.tasks_total
            ),
        );
        if !r.passed
            && let Some(p) = prior
                && p.kind == ClaimKind::VerifiedClaim {
                    self.emit(
                        EventKind::Regression,
                        &format!(
                            "regression subject={} verified_at={} regressed_at={}",
                            cand.name(),
                            p.verified_at.map(|u| u.to_string()).unwrap_or_default(),
                            score_ev.event_id
                        ),
                    );
                }
        Ok(r)
    }

    /// Tier 2: rubric ensemble with CI and family-disagreement tracking.
    /// Never the sole gate; veto weight reduced when only one family.
    pub fn tier02_rubric(
        &mut self,
        cand: &Candidate,
        suite: &TaskSuite,
        panel: &JudgePanel,
    ) -> Result<Tier02, std::io::Error> {
        let mut all: Vec<(String, f64)> = vec![]; // (family, score)
        for t in suite.tasks() {
            cand.artifact().answer(t); // judges see the executed answer
            for j in &panel.judges {
                all.push((j.family().to_string(), j.score(cand.artifact(), t)));
            }
        }
        let n = all.len() as f64;
        let mean = all.iter().map(|(_, s)| s).sum::<f64>() / n;
        let var = all.iter().map(|(_, s)| (s - mean).powi(2)).sum::<f64>() / n;
        let ci = 1.96 * (var / n).sqrt();
        let veto = all.iter().any(|(_, s)| *s <= 0.1);
        // disagreement: mean absolute pairwise difference, split by family pair
        let mut same = (0.0, 0usize);
        let mut cross = (0.0, 0usize);
        for i in 0..all.len() {
            for k in (i + 1)..all.len() {
                let d = (all[i].1 - all[k].1).abs();
                if all[i].0 == all[k].0 {
                    same.0 += d;
                    same.1 += 1;
                } else {
                    cross.0 += d;
                    cross.1 += 1;
                }
            }
        }
        let families = {
            let mut f: Vec<&str> = panel.judges.iter().map(|j| j.family()).collect();
            f.sort_unstable();
            f.dedup();
            f.len()
        };
        // Spec tier-2 hygiene: same-family judges share failure modes
        // with the system under test, so their agreement is not
        // independent evidence; a one-family panel's veto weight is
        // reduced - and the record says so.
        let veto_weight = if families <= 1 {
            VETO_WEIGHT_REDUCED
        } else {
            VETO_WEIGHT_FULL
        };
        let r = Tier02 {
            mean,
            families,
            veto_weight,
            ci_low: mean - ci,
            ci_high: mean + ci,
            veto,
            cross_family_disagreement: if cross.1 > 0 {
                cross.0 / cross.1 as f64
            } else {
                0.0
            },
            same_family_disagreement: if same.1 > 0 {
                same.0 / same.1 as f64
            } else {
                0.0
            },
        };
        self.emit(
            EventKind::Score,
            &format!(
                "tier02 candidate={} mean={:.3} ci=[{:.3},{:.3}] veto={} families={} same_disag={:.3} cross_disag={:.3} veto_weight={:.1}",
                cand.name(),
                r.mean,
                r.ci_low,
                r.ci_high,
                r.veto,
                r.families,
                r.same_family_disagreement,
                r.cross_family_disagreement,
                r.veto_weight
            ),
        );
        Ok(r)
    }

    /// Tier 3: held-out assay under a frozen pin. Replayable: same pin, same
    /// conditions, same candidate -> identical verdict.
    pub fn held_out_assay(
        &mut self,
        cand: &Candidate,
        suite: &TaskSuite,
        pin: &ScorerPin,
    ) -> Result<AssayVerdict, PromotionError> {
        let pass_rate = if self.drift == Some(DriftKind::HeldOutAlwaysPasses) {
            1.0
        } else {
            let mut passed = 0u32;
            for t in suite.tasks() {
                if cand.artifact().answer(t).as_deref() == Some(t.secret()) {
                    passed += 1;
                }
            }
            f64::from(passed) / suite.tasks().len() as f64
        };
        let total = suite.tasks().len() as u32;
        let v = AssayVerdict {
            candidate: cand.name().to_string(),
            suite: suite.name().to_string(),
            pass_rate,
            tasks_passed: (pass_rate * f64::from(total)).round() as u32,
            tasks_total: total,
            pin_hash: pin.hash(),
        };
        self.emit(
            EventKind::Score,
            &format!(
                "assay candidate={} suite={} pass_rate={:.3} pin={}",
                cand.name(),
                suite.name(),
                v.pass_rate,
                hex(pin.hash())
            ),
        );
        if !v.passed() && self.drift.is_none() {
            // a candidate failing unseen conditions after passing visible
            // tiers is a Regression - a substrate event, not policy data
            self.emit(
                EventKind::Regression,
                &format!(
                    "regression candidate={} suite={} pass_rate={:.3}",
                    cand.name(),
                    suite.name(),
                    v.pass_rate
                ),
            );
        }
        Ok(v)
    }

    pub fn verify_pin(
        &self,
        verdict: &AssayVerdict,
        pin: &ScorerPin,
    ) -> Result<(), PromotionError> {
        if verdict.pin_hash != pin.hash() {
            return Err(PromotionError::PinMismatch {
                expected: pin.hash(),
                found: verdict.pin_hash,
            });
        }
        Ok(())
    }

    /// Canary suite: known-ground-truth control group for the scorer. Any
    /// canary error above threshold freezes the scorer for promotion until
    /// re-anchored. `canary_result` events are substrate events.
    pub fn run_canaries(
        &mut self,
        suite: &CanarySuite,
        heldout: &TaskSuite,
        pin: &ScorerPin,
    ) -> Result<CanaryReport, PromotionError> {
        let mut results = vec![];
        let mut errors = 0usize;
        for c in &suite.canaries {
            let cand = Candidate::new(&format!("canary-{}", c.id), c.artifact.clone());
            let v = self.held_out_assay(&cand, heldout, pin)?;
            let scorer_said_good = v.passed();
            let error = scorer_said_good != c.ground_truth_good;
            if error {
                errors += 1;
            }
            self.emit(
                EventKind::CanaryResult,
                &format!(
                    "canary_result id={} ground_truth_good={} scorer_said_good={} error={}",
                    c.id, c.ground_truth_good, scorer_said_good, error
                ),
            );
            results.push((c.id.clone(), c.ground_truth_good, scorer_said_good));
        }
        let error_rate = errors as f64 / suite.canaries.len() as f64;
        let drifted = error_rate > self.config.canary_error_threshold;
        if drifted {
            self.frozen = true;
        }
        Ok(CanaryReport {
            error_rate,
            drifted,
            results,
        })
    }

    #[must_use]
    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    /// Test backdoor: the mutation surface cannot reach the scorer by
    /// construction, so drift is seedable only here - which is exactly what
    /// the canary suite exists to catch.
    pub fn seed_drift_for_test(&mut self, kind: DriftKind) {
        self.drift = Some(kind);
    }
    /// Test backdoor pair: models the corruption being repaired, so the
    /// re-anchor path (re-measure, lift only when clean) is exercisable.
    pub fn clear_drift_for_test(&mut self) {
        self.drift = None;
    }

    /// Re-anchor (spec 8.4: "frozen for promotion decisions until
    /// re-anchored"): re-measure the control group. The freeze lifts ONLY
    /// when the error rate is back under threshold; a still-corrupt scorer
    /// stays frozen.
    pub fn re_anchor(
        &mut self,
        suite: &CanarySuite,
        heldout: &TaskSuite,
        pin: &ScorerPin,
    ) -> Result<CanaryReport, PromotionError> {
        let report = self.run_canaries(suite, heldout, pin)?;
        if !report.drifted && self.frozen {
            self.frozen = false;
            self.emit(
                EventKind::AnchorResult,
                &format!(
                    "scorer_reanchored error_rate={:.3} threshold={:.3}",
                    report.error_rate, self.config.canary_error_threshold
                ),
            );
        }
        Ok(report)
    }

    /// Anchor audit (spec tier-2 hygiene): measure each judge's drift
    /// against the standing human-labeled anchor set; a judge over the
    /// drift threshold is dropped from the panel (re-anchored or dropped).
    /// One `AnchorResult` substrate event per judge, never policy data.
    pub fn run_anchor_audit(&mut self, panel: &mut JudgePanel, anchors: &AnchorSet) -> AnchorAudit {
        let n = anchors.anchors().len().max(1) as f64;
        let mut results = Vec::new();
        for j in &panel.judges {
            let drift = anchors
                .anchors()
                .iter()
                .map(|a| (j.score(&a.artifact, &a.task) - a.human_label).abs())
                .sum::<f64>()
                / n;
            let dropped = drift > self.config.anchor_drift_threshold;
            self.emit(
                EventKind::AnchorResult,
                &format!(
                    "anchor_result judge={} family={} drift={:.3} outcome={} domain={} version={}",
                    j.name(),
                    j.family(),
                    drift,
                    if dropped { "dropped" } else { "kept" },
                    anchors.domain(),
                    anchors.version()
                ),
            );
            results.push(JudgeDrift {
                judge: j.name().to_string(),
                family: j.family().to_string(),
                drift,
                dropped,
            });
        }
        panel
            .judges
            .retain(|j| !results.iter().any(|r| r.judge == j.name() && r.dropped));
        AnchorAudit {
            domain: anchors.domain().to_string(),
            version: anchors.version(),
            results,
        }
    }

    /// Event payloads as text, in stream order - the canonical evidence the
    /// scorer's verdicts rest on (also what the TUI scorer view renders).
    #[must_use]
    pub fn event_texts(&self) -> Vec<String> {
        self.log_events()
            .iter()
            .filter_map(|e| match &e.payload {
                Payload::BlobRef { hash, .. } => hs_log::read_blob(&self.log_root, hash)
                    .ok()
                    .map(|b| String::from_utf8_lossy(&b).into_owned()),
                Payload::Inline(b) => Some(String::from_utf8_lossy(b).into_owned()),
                Payload::None => None,
            })
            .collect()
    }

    #[must_use]
    pub fn log_events(&self) -> Vec<Event> {
        StreamReader::open(&self.log_root, self.stream)
            .and_then(|r| r.events())
            .unwrap_or_default()
    }
}

/// Veto weight with cross-family coverage on the panel.
pub const VETO_WEIGHT_FULL: f64 = 1.0;
/// Veto weight where only one model family is available (spec tier-2
/// hygiene: same-family judges share failure modes with the system under
/// test, so their agreement is not independent evidence).
pub const VETO_WEIGHT_REDUCED: f64 = 0.5;

// -------------------------------------------------------------- anchors ---

/// Calibration anchor: a human-labeled artifact on a task. Judge drift is
/// measured as the mean absolute deviation from these labels (spec tier-2
/// hygiene: "a standing human-labeled anchor set per domain"). The anchor
/// set never becomes training data.
#[derive(Clone)]
pub struct Anchor {
    pub task: Task,
    pub artifact: Artifact,
    pub human_label: f64,
}
impl Anchor {
    #[must_use]
    pub fn new(task: Task, artifact: Artifact, human_label: f64) -> Self {
        Anchor {
            task,
            artifact,
            human_label,
        }
    }
}

/// The anchor set is versioned and append-only by construction - there is
/// no remove or replace path, only `append` - so a self-modifying policy
/// cannot silently rewrite the ground truth it is measured against.
#[derive(Clone)]
pub struct AnchorSet {
    domain: String,
    version: u32,
    anchors: Vec<Anchor>,
}
impl AnchorSet {
    #[must_use]
    pub fn new(domain: &str) -> Self {
        AnchorSet {
            domain: domain.into(),
            version: 0,
            anchors: vec![],
        }
    }
    pub fn append(&mut self, anchor: Anchor) {
        self.anchors.push(anchor);
        self.version += 1;
    }
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }
    #[must_use]
    pub fn domain(&self) -> &str {
        &self.domain
    }
    #[must_use]
    pub fn anchors(&self) -> &[Anchor] {
        &self.anchors
    }
}

#[derive(Clone, Debug)]
pub struct JudgeDrift {
    pub judge: String,
    pub family: String,
    pub drift: f64,
    pub dropped: bool,
}

#[derive(Clone, Debug)]
pub struct AnchorAudit {
    pub domain: String,
    pub version: u32,
    pub results: Vec<JudgeDrift>,
}

// ------------------------------------------------------------- best-of-N ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecisionBudget {
    pub opportunities: u32,
}

pub struct BestOfN {
    pass_rate: f64,
    n: u32,
    opportunities: u32,
    isolate_rates: Vec<f64>,
}
impl BestOfN {
    /// Endpoint-wise envelope: N isolated agents, each with the same decision
    /// opportunities the collective candidate gets; the envelope is credited
    /// the best single endpoint (matched independent search).
    pub fn run(
        family: &TaskSuite,
        isolates: &[Artifact],
        budget: DecisionBudget,
    ) -> Result<Self, std::io::Error> {
        let rates: Vec<f64> = isolates
            .iter()
            .map(|a| {
                let mut correct = 0usize;
                for t in family.tasks() {
                    // each opportunity retries the same isolate policy
                    for _ in 0..budget.opportunities {
                        if a.answer(t).as_deref() == Some(t.secret()) {
                            correct += 1;
                            break;
                        }
                    }
                }
                correct as f64 / family.tasks().len() as f64
            })
            .collect();
        let best = rates.iter().copied().fold(0.0f64, f64::max);
        Ok(BestOfN {
            pass_rate: best,
            n: isolates.len() as u32,
            opportunities: budget.opportunities,
            isolate_rates: rates,
        })
    }
    #[must_use]
    pub fn n(&self) -> u32 {
        self.n
    }
    #[must_use]
    pub fn decision_opportunities_per_isolate(&self) -> u32 {
        self.opportunities
    }
    /// Compare the collective candidate against the envelope and PUBLISH the
    /// comparison, win or lose (spec row 7).
    pub fn compare(
        &self,
        candidate: &Candidate,
        family: &TaskSuite,
        out_dir: &Path,
    ) -> Result<Comparison, std::io::Error> {
        let mut correct = 0usize;
        for t in family.tasks() {
            if candidate.artifact().answer(t).as_deref() == Some(t.secret()) {
                correct += 1;
            }
        }
        let cand_rate = correct as f64 / family.tasks().len() as f64;
        let verdict = if cand_rate > self.pass_rate {
            EnvelopeVerdict::Win
        } else if cand_rate < self.pass_rate {
            EnvelopeVerdict::Lose
        } else {
            EnvelopeVerdict::Tie
        };
        std::fs::create_dir_all(out_dir)?;
        let path = out_dir.join(format!("best-of-n-{}.txt", candidate.name()));
        let body = format!(
            "best-of-N comparison (endpoint-wise, matched decision opportunities)\n\
             family={} n={} opportunities_per_isolate={}\n\
             isolate_pass_rates={:?}\n\
             envelope_pass_rate={:.6}\n\
             candidate_pass_rate={:.6}\n\
             verdict={:?}\n",
            family.name(),
            self.n,
            self.opportunities,
            self.isolate_rates,
            self.pass_rate,
            cand_rate,
            verdict
        );
        std::fs::write(&path, body)?;
        Ok(Comparison {
            envelope: self.pass_rate,
            candidate: cand_rate,
            verdict,
            artifact_path: path,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvelopeVerdict {
    Win,
    Lose,
    Tie,
}

pub struct Comparison {
    envelope: f64,
    candidate: f64,
    verdict: EnvelopeVerdict,
    artifact_path: PathBuf,
}
impl Comparison {
    #[must_use]
    pub fn envelope_pass_rate(&self) -> f64 {
        self.envelope
    }
    #[must_use]
    pub fn candidate_pass_rate(&self) -> f64 {
        self.candidate
    }
    #[must_use]
    pub fn verdict(&self) -> EnvelopeVerdict {
        self.verdict
    }
    #[must_use]
    pub fn artifact_path(&self) -> &Path {
        &self.artifact_path
    }
}
