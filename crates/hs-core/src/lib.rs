//! HAIRSPRING gate 1 - canonical event schema (engineering design v4, section 3).
//!
//! One append-only, hash-chained log per stream. The encoding below is the
//! canonical record: deterministic, versioned by construction (unknown tags
//! are rejected), and the only bytes the hash chain covers. Floats never
//! appear: cost is decimal micro-USD, time is epoch millis.

use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Event kinds from spec section 3, plus kinds reserved 2026-09-02 for
/// gates 7-8 so no schema migration is needed later:
/// - CapabilityDelta / FitnessDelta: scorer must distinguish a score change
///   caused by swapping the model/harness (capability) from evolved-fitness
///   improvement (fitness); they are different event kinds.
/// - Regression: "verified at event N, regressed at event M" bookkeeping.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum EventKind {
    ModelCall = 0,
    ToolCall = 1,
    Observation = 2,
    Decision = 3,
    ContextInject = 4,
    Feedback = 5,
    SnapshotRef = 6,
    Proposal = 7,
    Consequence = 8,
    GoalUpdate = 9,
    BudgetUpdate = 10,
    Spawn = 11,
    Message = 12,
    Mutation = 13,
    Score = 14,
    ScorerPin = 15,
    CanaryResult = 16,
    Prefetch = 17,
    // 18..=63 unassigned spec space.
    CapabilityDelta = 64, // reserved: gates 7-8
    FitnessDelta = 65,    // reserved: gates 7-8
    Regression = 66,      // reserved: gates 7-8
}

impl EventKind {
    pub fn tag(self) -> u8 {
        self as u8
    }
    pub fn from_tag(tag: u8) -> Result<Self, DecodeError> {
        Ok(match tag {
            0 => Self::ModelCall,
            1 => Self::ToolCall,
            2 => Self::Observation,
            3 => Self::Decision,
            4 => Self::ContextInject,
            5 => Self::Feedback,
            6 => Self::SnapshotRef,
            7 => Self::Proposal,
            8 => Self::Consequence,
            9 => Self::GoalUpdate,
            10 => Self::BudgetUpdate,
            11 => Self::Spawn,
            12 => Self::Message,
            13 => Self::Mutation,
            14 => Self::Score,
            15 => Self::ScorerPin,
            16 => Self::CanaryResult,
            17 => Self::Prefetch,
            64 => Self::CapabilityDelta,
            65 => Self::FitnessDelta,
            66 => Self::Regression,
            other => return Err(DecodeError::UnknownKindTag(other)),
        })
    }
}

/// Typed payload union. Large bodies live in the artifact (blob) store and
/// are referenced by content hash (spec: payload-by-hash).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Payload {
    None,
    Inline(Vec<u8>),
    BlobRef { hash: [u8; 32], len: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub event_id: Uuid,
    pub stream_id: Uuid,
    pub seq: u64,
    pub ts_wall_ms: i64,
    pub kind: EventKind,
    pub payload: Payload,
    pub parent_event_id: Option<Uuid>,
    pub latency_ms: u32,
    /// Decimal cost in micro-USD (1e-6 USD). Never a float.
    pub cost_usd_micros: i64,
    pub sandbox_snap_id: Option<Uuid>,
    pub prev_hash: [u8; 32],
    pub hash: [u8; 32],
}

#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    Truncated { at: &'static str },
    UnknownKindTag(u8),
    UnknownPayloadTag(u8),
    TrailingBytes(usize),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated { at } => write!(f, "truncated encoding at {at}"),
            Self::UnknownKindTag(t) => write!(f, "unknown event kind tag {t:#04x}"),
            Self::UnknownPayloadTag(t) => write!(f, "unknown payload tag {t:#04x}"),
            Self::TrailingBytes(n) => write!(f, "{n} trailing bytes after event"),
        }
    }
}
impl std::error::Error for DecodeError {}

struct Writer(Vec<u8>);
impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v)
    }
    fn u32(&mut self, v: u32) {
        self.0.extend(v.to_le_bytes())
    }
    fn u64(&mut self, v: u64) {
        self.0.extend(v.to_le_bytes())
    }
    fn i64(&mut self, v: i64) {
        self.0.extend(v.to_le_bytes())
    }
    fn raw(&mut self, v: &[u8]) {
        self.0.extend(v)
    }
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize, at: &'static str) -> Result<&'a [u8], DecodeError> {
        if self.pos + n > self.buf.len() {
            return Err(DecodeError::Truncated { at });
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self, at: &'static str) -> Result<u8, DecodeError> {
        Ok(self.take(1, at)?[0])
    }
    fn u32(&mut self, at: &'static str) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.take(4, at)?.try_into().unwrap()))
    }
    fn u64(&mut self, at: &'static str) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.take(8, at)?.try_into().unwrap()))
    }
    fn i64(&mut self, at: &'static str) -> Result<i64, DecodeError> {
        Ok(i64::from_le_bytes(self.take(8, at)?.try_into().unwrap()))
    }
}

impl Event {
    /// Canonical encoding of every field including `hash`.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = self.encode_body();
        w.raw(&self.hash);
        w.0
    }

    /// Canonical encoding of every field except `hash`; the bytes the chain
    /// hash is computed over.
    fn encode_body(&self) -> Writer {
        let mut w = Writer(Vec::with_capacity(128));
        w.raw(self.event_id.as_bytes());
        w.raw(self.stream_id.as_bytes());
        w.u64(self.seq);
        w.i64(self.ts_wall_ms);
        w.u8(self.kind.tag());
        match &self.payload {
            Payload::None => w.u8(0),
            Payload::Inline(b) => {
                w.u8(1);
                w.u32(b.len() as u32);
                w.raw(b);
            }
            Payload::BlobRef { hash, len } => {
                w.u8(2);
                w.raw(hash);
                w.u64(*len);
            }
        }
        match &self.parent_event_id {
            Some(id) => {
                w.u8(1);
                w.raw(id.as_bytes());
            }
            None => w.u8(0),
        }
        w.u32(self.latency_ms);
        w.i64(self.cost_usd_micros);
        match &self.sandbox_snap_id {
            Some(id) => {
                w.u8(1);
                w.raw(id.as_bytes());
            }
            None => w.u8(0),
        }
        w.raw(&self.prev_hash);
        w
    }

    pub fn compute_hash(&self) -> [u8; 32] {
        Sha256::digest(&self.encode_body().0).into()
    }

    pub fn verify_hash(&self) -> bool {
        self.compute_hash() == self.hash
    }

    pub fn decode(buf: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader { buf, pos: 0 };
        let event_id = Uuid::from_bytes(r.take(16, "event_id")?.try_into().unwrap());
        let stream_id = Uuid::from_bytes(r.take(16, "stream_id")?.try_into().unwrap());
        let seq = r.u64("seq")?;
        let ts_wall_ms = r.i64("ts_wall_ms")?;
        let kind = EventKind::from_tag(r.u8("kind")?)?;
        let payload = match r.u8("payload")? {
            0 => Payload::None,
            1 => {
                let n = r.u32("payload.len")? as usize;
                Payload::Inline(r.take(n, "payload.bytes")?.to_vec())
            }
            2 => {
                let hash: [u8; 32] = r.take(32, "payload.hash")?.try_into().unwrap();
                let len = r.u64("payload.len")?;
                Payload::BlobRef { hash, len }
            }
            t => return Err(DecodeError::UnknownPayloadTag(t)),
        };
        let parent_event_id = match r.u8("parent")? {
            0 => None,
            _ => Some(Uuid::from_bytes(
                r.take(16, "parent.id")?.try_into().unwrap(),
            )),
        };
        let latency_ms = r.u32("latency_ms")?;
        let cost_usd_micros = r.i64("cost_usd_micros")?;
        let sandbox_snap_id = match r.u8("snap")? {
            0 => None,
            _ => Some(Uuid::from_bytes(r.take(16, "snap.id")?.try_into().unwrap())),
        };
        let prev_hash: [u8; 32] = r.take(32, "prev_hash")?.try_into().unwrap();
        let hash: [u8; 32] = r.take(32, "hash")?.try_into().unwrap();
        if r.pos != buf.len() {
            return Err(DecodeError::TrailingBytes(buf.len() - r.pos));
        }
        Ok(Event {
            event_id,
            stream_id,
            seq,
            ts_wall_ms,
            kind,
            payload,
            parent_event_id,
            latency_ms,
            cost_usd_micros,
            sandbox_snap_id,
            prev_hash,
            hash,
        })
    }
}

/// Builder for the fields an appender chooses. The log writer assigns seq,
/// prev_hash, and hash; nothing else may.
pub struct EventBuilder {
    event: Event,
}
impl EventBuilder {
    pub fn new(kind: EventKind) -> Self {
        EventBuilder {
            event: Event {
                event_id: Uuid::nil(),
                stream_id: Uuid::nil(),
                seq: 0,
                ts_wall_ms: 0,
                kind,
                payload: Payload::None,
                parent_event_id: None,
                latency_ms: 0,
                cost_usd_micros: 0,
                sandbox_snap_id: None,
                prev_hash: [0; 32],
                hash: [0; 32],
            },
        }
    }
    pub fn payload(mut self, p: Payload) -> Self {
        self.event.payload = p;
        self
    }
    pub fn parent(mut self, id: Uuid) -> Self {
        self.event.parent_event_id = Some(id);
        self
    }
    pub fn latency_ms(mut self, v: u32) -> Self {
        self.event.latency_ms = v;
        self
    }
    pub fn cost_usd_micros(mut self, v: i64) -> Self {
        self.event.cost_usd_micros = v;
        self
    }
    pub fn sandbox_snap(mut self, id: Uuid) -> Self {
        self.event.sandbox_snap_id = Some(id);
        self
    }
    pub fn ts_wall_ms(mut self, v: i64) -> Self {
        self.event.ts_wall_ms = v;
        self
    }
    /// Partial build for schema/unit tests: ids, seq, and chain fields left unset.
    pub fn build_part(self) -> Event {
        self.event
    }
    /// Full build used by the log writer after it assigns chain fields.
    pub fn build(self) -> Event {
        self.event
    }
}

/// Test-only helpers exposing encoding layout without making it public API.
pub mod testing {
    /// Byte offset of the kind tag in the canonical encoding:
    /// 16 (event_id) + 16 (stream_id) + 8 (seq) + 8 (ts_wall_ms).
    pub fn kind_tag_offset(_encoded: &[u8]) -> usize {
        16 + 16 + 8 + 8
    }
}
