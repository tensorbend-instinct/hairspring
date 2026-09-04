//! GATE 9b (spec v5; Zhao & Zhao, arXiv:2609.00546 - v1 preprint, imports
//! carried as design rules): substrate swaps are TRANSACTIONS, continuation
//! authority is FENCED.
//!
//! A model, executor-build, harness, or host swap runs quiesce -> checkpoint
//! -> validate -> bind -> rehydrate -> resume with a single promotion point
//! (bind). The old variant is fenced the moment the new one binds; a failed
//! transaction leaves the old variant in authority. Every step lands on the
//! canonical log as a capability_change event carrying both binding refs and
//! the protocol step, so a swap is never invisible to the scorer, and the
//! in-flight state is recoverable from the log alone (memory is a read path
//! over the log - no parallel store).
//!
//! Fencing: at most one binding holds continuation authority over a stream
//! at a time. A second claimant is a fencing violation, not a race to
//! tolerate. A fork under assay never holds authority; promotion transfers
//! it exactly once, at bind.

use hs_core::{EventBuilder, EventKind, Payload};
use hs_log::{StreamReader, StreamWriter};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use uuid::Uuid;

/// A replaceable substrate binding: the model, the executor build, the
/// harness, or the host. The continuity substrate (identity + memory + the
/// versioned body = policy + log) is what survives the swap.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub kind: BindingKind,
    pub reference: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindingKind {
    Model,
    Harness,
    Executor,
    Host,
}

impl Binding {
    pub fn model(reference: &str) -> Self {
        Self { kind: BindingKind::Model, reference: reference.into() }
    }
    pub fn harness(reference: &str) -> Self {
        Self { kind: BindingKind::Harness, reference: reference.into() }
    }
    pub fn executor(reference: &str) -> Self {
        Self { kind: BindingKind::Executor, reference: reference.into() }
    }
    pub fn host(reference: &str) -> Self {
        Self { kind: BindingKind::Host, reference: reference.into() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationStep {
    Quiesce,
    Checkpoint,
    Validate,
    Bind,
    Rehydrate,
    Resume,
    ValidateFailed,
    Abort,
}

impl MigrationStep {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Quiesce => "quiesce",
            Self::Checkpoint => "checkpoint",
            Self::Validate => "validate",
            Self::Bind => "bind",
            Self::Rehydrate => "rehydrate",
            Self::Resume => "resume",
            Self::ValidateFailed => "validate_failed",
            Self::Abort => "abort",
        }
    }
    fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "quiesce" => Self::Quiesce,
            "checkpoint" => Self::Checkpoint,
            "validate" => Self::Validate,
            "bind" => Self::Bind,
            "rehydrate" => Self::Rehydrate,
            "resume" => Self::Resume,
            "validate_failed" => Self::ValidateFailed,
            "abort" => Self::Abort,
            _ => return None,
        })
    }
    fn terminal(&self) -> bool {
        matches!(self, Self::Resume | Self::Abort)
    }
}

#[derive(Debug)]
pub enum MigrationError {
    /// Two live bindings acting as the same run. Never tolerated.
    FencingViolation(String),
    ValidationFailed(String),
    InvalidTransition(String),
    Log(hs_log::LogError),
}

impl std::fmt::Display for MigrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FencingViolation(m) => write!(f, "fencing violation: {m}"),
            Self::ValidationFailed(m) => write!(f, "validation failed: {m}"),
            Self::InvalidTransition(m) => write!(f, "invalid transition: {m}"),
            Self::Log(e) => write!(f, "log: {e:?}"),
        }
    }
}
impl std::error::Error for MigrationError {}
impl From<hs_log::LogError> for MigrationError {
    fn from(e: hs_log::LogError) -> Self {
        Self::Log(e)
    }
}

/// Single-authority fencing over streams: at most one binding holds
/// continuation authority at a time. In-memory by construction; the durable
/// record is the log (bind/abort events), and recovery replays it.
#[derive(Default)]
pub struct ContinuityAuthority {
    holders: HashMap<Uuid, Binding>,
}

impl ContinuityAuthority {
    pub fn new() -> Self {
        Self::default()
    }

    /// Claim authority for a binding. Idempotent for the current holder; a
    /// different claimant on a held stream is a fencing violation.
    pub fn claim(&mut self, stream: Uuid, b: Binding) -> Result<(), MigrationError> {
        match self.holders.get(&stream) {
            Some(h) if *h != b => Err(MigrationError::FencingViolation(format!(
                "stream {stream} held by {:?}; {:?} may not claim",
                h.reference, b.reference
            ))),
            _ => {
                self.holders.insert(stream, b);
                Ok(())
            }
        }
    }

    pub fn holds(&self, stream: Uuid, b: &Binding) -> bool {
        self.holders.get(&stream) == Some(b)
    }

    /// Acting on a stream without holding its authority is a violation.
    pub fn assert_holder(&self, stream: Uuid, b: &Binding) -> Result<(), MigrationError> {
        if self.holds(stream, b) {
            Ok(())
        } else {
            Err(MigrationError::FencingViolation(format!(
                "{:?} does not hold continuation authority over stream {stream}",
                b.reference
            )))
        }
    }
}

/// One swap transaction. Owns the stream writer; state is derived from (and
/// always flushed to) the canonical log before any method returns.
pub struct Migration {
    stream: Uuid,
    old: Binding,
    new: Binding,
    step: MigrationStep,
    writer: StreamWriter,
    bound: bool,
}

impl Migration {
    /// Quiesce the old variant and open the transaction.
    pub fn begin(
        log_root: &Path,
        stream: Uuid,
        old: Binding,
        new: Binding,
    ) -> Result<Self, MigrationError> {
        let writer = StreamWriter::create(log_root, stream)?;
        let mut m = Self {
            stream,
            old,
            new,
            step: MigrationStep::Quiesce,
            writer,
            bound: false,
        };
        m.emit(MigrationStep::Quiesce)?;
        Ok(m)
    }

    pub fn old(&self) -> &Binding {
        &self.old
    }
    pub fn new(&self) -> &Binding {
        &self.new
    }
    pub fn step(&self) -> MigrationStep {
        self.step
    }

    fn emit(&mut self, step: MigrationStep) -> Result<(), MigrationError> {
        let body = format!(
            "{{\"old_binding\":\"{}\",\"new_binding\":\"{}\",\"step\":\"{}\"}}",
            self.old.reference,
            self.new.reference,
            step.as_str()
        );
        self.writer.append(
            EventBuilder::new(EventKind::CapabilityChange).payload(Payload::Inline(body.into_bytes())),
        )?;
        self.step = step;
        Ok(())
    }

    fn expect(&self, from: MigrationStep, to: &'static str) -> Result<(), MigrationError> {
        if self.step != from {
            return Err(MigrationError::InvalidTransition(format!(
                "{} requires state {:?}, at {:?}",
                to,
                from,
                self.step
            )));
        }
        Ok(())
    }

    pub fn checkpoint(&mut self) -> Result<(), MigrationError> {
        self.expect(MigrationStep::Quiesce, "checkpoint")?;
        self.emit(MigrationStep::Checkpoint)
    }

    /// Validate the checkpoint. A false verdict is NOT silent: validate_failed
    /// lands on the log and the transaction must abort - the old variant
    /// keeps authority throughout.
    pub fn validate(&mut self, f: impl FnOnce() -> bool) -> Result<(), MigrationError> {
        self.expect(MigrationStep::Checkpoint, "validate")?;
        if f() {
            self.emit(MigrationStep::Validate)
        } else {
            self.emit(MigrationStep::ValidateFailed)?;
            Err(MigrationError::ValidationFailed(format!(
                "new binding {:?} failed validation",
                self.new.reference
            )))
        }
    }

    /// The single promotion point: authority moves old -> new exactly here,
    /// exactly once. The old variant is fenced from this moment.
    pub fn bind(&mut self, auth: &mut ContinuityAuthority) -> Result<(), MigrationError> {
        self.expect(MigrationStep::Validate, "bind")?;
        if self.bound {
            return Err(MigrationError::FencingViolation(
                "promotion point may be taken exactly once".into(),
            ));
        }
        // fence the old, seat the new - as one step, before the event lands
        auth.claim(self.stream, self.new.clone()).or_else(|e| {
            // held by the old variant: that is the expected pre-bind state
            if auth.holds(self.stream, &self.old) {
                auth.holders.insert(self.stream, self.new.clone());
                Ok(())
            } else {
                Err(e)
            }
        })?;
        self.bound = true;
        self.emit(MigrationStep::Bind)
    }

    pub fn rehydrate(&mut self) -> Result<(), MigrationError> {
        if self.step != MigrationStep::Bind {
            return Err(MigrationError::InvalidTransition(format!(
                "rehydrate requires state Bind, at {:?}",
                self.step
            )));
        }
        self.emit(MigrationStep::Rehydrate)
    }

    pub fn resume(&mut self) -> Result<(), MigrationError> {
        if self.step != MigrationStep::Rehydrate {
            return Err(MigrationError::InvalidTransition(format!(
                "resume requires state Rehydrate, at {:?}",
                self.step
            )));
        }
        self.emit(MigrationStep::Resume)
    }

    /// Resume against an authority rebuilt after recovery: the new binding
    /// (already seated at bind) is re-claimed into the fresh registry.
    pub fn resume_with(&mut self, auth: &mut ContinuityAuthority) -> Result<(), MigrationError> {
        if self.step == MigrationStep::Rehydrate && !auth.holds(self.stream, &self.new) {
            auth.claim(self.stream, self.new.clone())?;
        }
        self.resume()
    }

    /// A failed transaction leaves the old variant in authority.
    pub fn abort(&mut self, auth: &mut ContinuityAuthority) -> Result<(), MigrationError> {
        if self.step.terminal() {
            return Err(MigrationError::InvalidTransition(format!(
                "transaction already {:?}",
                self.step
            )));
        }
        if auth.holds(self.stream, &self.new) {
            auth.holders.insert(self.stream, self.old.clone());
        } else if !auth.holds(self.stream, &self.old) {
            auth.claim(self.stream, self.old.clone())?;
        }
        self.emit(MigrationStep::Abort)
    }

    /// Rebuild the in-flight transaction from the log alone. Returns None
    /// when no transaction is open (none started, or the last reached a
    /// terminal step). Memory is a read path over the log: no side store.
    pub fn recover(log_root: &Path, stream: Uuid) -> Option<Self> {
        let events = StreamReader::open(log_root, stream).and_then(|r| r.events()).ok()?;
        let mut old: Option<Binding> = None;
        let mut new: Option<Binding> = None;
        let mut last: Option<MigrationStep> = None;
        for e in &events {
            if e.kind != EventKind::CapabilityChange {
                continue;
            }
            let Payload::Inline(b) = &e.payload else { continue };
            let v: serde_json::Value = serde_json::from_slice(b).ok()?;
            // a fresh quiesce opens a new transaction; later steps extend it
            let step = MigrationStep::from_str(v["step"].as_str()?)?;
            if step == MigrationStep::Quiesce || old.is_none() {
                old = Some(Binding::model(v["old_binding"].as_str()?));
                new = Some(Binding::model(v["new_binding"].as_str()?));
            }
            last = Some(step);
        }
        let last = last?;
        if last.terminal() {
            return None;
        }
        let writer = StreamWriter::resume(log_root, stream).ok()?.writer;
        Some(Self {
            stream,
            old: old?,
            new: new?,
            step: last,
            writer,
            bound: last == MigrationStep::Bind
                || last == MigrationStep::Rehydrate,
        })
    }
}
