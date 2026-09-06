//! HAIRSPRING gate 1 - the canonical append-only event log (spec section 3).
//!
//! On-disk layout under the log root:
//!   streams/<stream_uuid>/seg-NNNNNN.hslog   frames, append-only
//!   blobs/<hex[0..2]>/<hex[2..4]>/<sha256hex>  payload bodies, content-addressed
//!
//! Frame = [u32 LE body_len][u32 LE crc32(body)][body], body = canonical
//! event encoding from hs-core (hash included). Durability: every append is
//! fsynced before it is acknowledged - "zero state loss on SIGKILL" is the
//! gate-1 proof, so batched fsync (spec section 7, a hot-path optimization)
//! is deliberately not built at this gate.

use hs_core::{Event, EventBuilder, Payload};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Payloads larger than this live in the blob store, referenced by hash.
pub const INLINE_CAP: usize = 4096;
/// Segment rollover target.
pub const SEGMENT_MAX_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug)]
pub enum LogError {
    Io(io::Error),
    StreamExists(Uuid),
    StreamMissing(Uuid),
    Decode(hs_core::DecodeError),
    Corruption(Corruption),
}
impl From<io::Error> for LogError {
    fn from(e: io::Error) -> Self {
        LogError::Io(e)
    }
}
impl From<hs_core::DecodeError> for LogError {
    fn from(e: hs_core::DecodeError) -> Self {
        LogError::Decode(e)
    }
}
impl std::fmt::Display for LogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::StreamExists(id) => write!(f, "stream {id} already exists"),
            Self::StreamMissing(id) => write!(f, "stream {id} not found"),
            Self::Decode(e) => write!(f, "decode: {e}"),
            Self::Corruption(c) => write!(f, "corruption at seq {}: {:?}", c.seq, c.kind),
        }
    }
}
impl std::error::Error for LogError {}

#[derive(Debug)]
pub struct Corruption {
    pub seq: u64,
    pub kind: CorruptionKind,
}
#[derive(Debug)]
pub enum CorruptionKind {
    CrcMismatch,
    Decode(String),
    HashMismatch,
    PrevHashMismatch { expected: [u8; 32], found: [u8; 32] },
    SeqGap { expected: u64, found: u64 },
    BlobMissing { hash: [u8; 32] },
    BlobHashMismatch { hash: [u8; 32] },
    TruncatedFrameMidStream,
}

#[derive(Debug)]
pub struct VerifyReport {
    pub events: u64,
    pub last_hash: [u8; 32],
}

fn stream_dir(root: &Path, stream: Uuid) -> PathBuf {
    root.join("streams").join(stream.to_string())
}
fn blob_path_inner(root: &Path, hash: &[u8; 32]) -> PathBuf {
    let hex = hex_encode(hash);
    root.join("blobs")
        .join(&hex[0..2])
        .join(&hex[2..4])
        .join(hex)
}
fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn segment_files(root: &Path, stream: Uuid) -> io::Result<Vec<PathBuf>> {
    let dir = stream_dir(root, stream);
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "hslog").unwrap_or(false))
        .collect();
    files.sort();
    Ok(files)
}
fn fsync_dir(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn write_frame(file: &mut File, body: &[u8]) -> io::Result<u64> {
    let len = body.len() as u32;
    let crc = crc32fast::hash(body);
    let mut frame = Vec::with_capacity(8 + body.len());
    frame.extend(len.to_le_bytes());
    frame.extend(crc.to_le_bytes());
    frame.extend(body);
    file.write_all(&frame)?;
    file.sync_all()?; // durable before ack: the gate-1 contract
    Ok(frame.len() as u64)
}

enum FrameRead {
    Ok(Event, u64), // event, total frame bytes
    TornTail(u64),  // bytes present but incomplete
    Corrupt(Corruption),
    End,
}

fn read_frame(file: &mut File) -> Result<FrameRead, LogError> {
    let mut hdr = [0u8; 8];
    let n = read_up_to(file, &mut hdr)?;
    if n == 0 {
        return Ok(FrameRead::End);
    }
    if n < 8 {
        return Ok(FrameRead::TornTail(n as u64));
    }
    let len = u32::from_le_bytes(hdr[0..4].try_into().unwrap()) as usize;
    let crc = u32::from_le_bytes(hdr[4..8].try_into().unwrap());
    // Absurd length means a torn/garbage header, not a 4GB allocation.
    if len as u64 > SEGMENT_MAX_BYTES {
        return Ok(FrameRead::TornTail(8));
    }
    let mut body = vec![0u8; len];
    let n = read_up_to(file, &mut body)?;
    if n < len {
        return Ok(FrameRead::TornTail(8 + n as u64));
    }
    if crc32fast::hash(&body) != crc {
        // CRC failed: we cannot trust the seq field, so report at the
        // position the caller tracks.
        return Ok(FrameRead::Corrupt(Corruption {
            seq: u64::MAX,
            kind: CorruptionKind::CrcMismatch,
        }));
    }
    match Event::decode(&body) {
        Ok(e) => Ok(FrameRead::Ok(e, 8 + len as u64)),
        Err(e) => Ok(FrameRead::Corrupt(Corruption {
            seq: u64::MAX,
            kind: CorruptionKind::Decode(e.to_string()),
        })),
    }
}

fn read_up_to(f: &mut File, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(m) => n += m,
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

pub struct StreamWriter {
    root: PathBuf,
    stream: Uuid,
    file: File,
    seg_index: usize,
    seg_len: u64,
    next_seq: u64,
    last_hash: [u8; 32],
    last_event_id: Option<Uuid>,
}

pub struct ResumeOutcome {
    pub writer: StreamWriter,
    pub events_recovered: u64,
    pub truncated_bytes: u64,
}

impl StreamWriter {
    pub fn create(root: &Path, stream: Uuid) -> Result<Self, LogError> {
        let dir = stream_dir(root, stream);
        if dir.exists() {
            return Err(LogError::StreamExists(stream));
        }
        fs::create_dir_all(&dir)?;
        fsync_dir(&dir.join("..").canonicalize().unwrap_or(dir.clone()))?;
        let file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(dir.join("seg-000000.hslog"))?;
        fsync_dir(&dir)?;
        Ok(StreamWriter {
            root: root.to_path_buf(),
            stream,
            file,
            seg_index: 0,
            seg_len: 0,
            next_seq: 0,
            last_hash: [0; 32],
            last_event_id: None,
        })
    }

    /// Reopen a stream after interruption: validate the whole chain, then
    /// truncate a torn tail left by a mid-write kill. History is never
    /// rewritten - recovery only cuts bytes that were never acknowledged.
    pub fn resume(root: &Path, stream: Uuid) -> Result<ResumeOutcome, LogError> {
        let files = segment_files(root, stream)?;
        if files.is_empty() {
            return Err(LogError::StreamMissing(stream));
        }
        let mut next_seq = 0u64;
        let mut last_hash = [0u8; 32];
        let mut last_event_id = None;
        let mut recovered = 0u64;
        let mut truncated = 0u64;
        let mut last_seg_index = 0usize;
        let mut last_seg_len = 0u64;

        for (idx, path) in files.iter().enumerate() {
            let mut f = File::open(path)?;
            let mut offset = 0u64;
            let file_len = f.metadata()?.len();
            loop {
                match read_frame(&mut f)? {
                    FrameRead::End => break,
                    FrameRead::TornTail(n) => {
                        let is_last = idx == files.len() - 1;
                        if !is_last {
                            return Err(LogError::Corruption(Corruption {
                                seq: next_seq,
                                kind: CorruptionKind::TruncatedFrameMidStream,
                            }));
                        }
                        let keep = offset;
                        let total = file_len;
                        drop(f);
                        let wf = OpenOptions::new().write(true).open(path)?;
                        wf.set_len(keep)?;
                        wf.sync_all()?;
                        truncated = total - keep;
                        let _ = n;
                        break;
                    }
                    FrameRead::Corrupt(mut c) => {
                        c.seq = next_seq;
                        return Err(LogError::Corruption(c));
                    }
                    FrameRead::Ok(e, n) => {
                        check_chain(&e, next_seq, last_hash)?;
                        next_seq = e.seq + 1;
                        last_hash = e.hash;
                        last_event_id = Some(e.event_id);
                        recovered += 1;
                        offset += n;
                    }
                }
            }
            if truncated > 0 {
                last_seg_index = idx;
                last_seg_len = offset;
                break; // everything after the torn tail is unreachable
            }
            last_seg_index = idx;
            last_seg_len = offset;
        }

        let path = &files[last_seg_index];
        let file = OpenOptions::new().append(true).open(path)?;
        Ok(ResumeOutcome {
            writer: StreamWriter {
                root: root.to_path_buf(),
                stream,
                file,
                seg_index: last_seg_index,
                seg_len: last_seg_len,
                next_seq,
                last_hash,
                last_event_id,
            },
            events_recovered: recovered,
            truncated_bytes: truncated,
        })
    }

    pub fn last_hash(&self) -> [u8; 32] {
        self.last_hash
    }
    pub fn last_event_id(&self) -> Option<Uuid> {
        self.last_event_id
    }
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Append one event. The writer assigns seq, prev_hash, and hash; the
    /// caller owns everything else. Durable (fsynced) when returned.
    pub fn append(&mut self, b: EventBuilder) -> Result<Event, LogError> {
        let mut e = b.build();
        e.event_id = Uuid::new_v4();
        e.stream_id = self.stream;
        e.seq = self.next_seq;
        if e.ts_wall_ms == 0 {
            e.ts_wall_ms = now_ms();
        }
        e.prev_hash = self.last_hash;
        // Payload-by-hash: large bodies move to the blob store (spec 3).
        if let Payload::Inline(bytes) = &e.payload {
            if bytes.len() > INLINE_CAP {
                let hash = self.store_blob(bytes)?;
                e.payload = Payload::BlobRef {
                    hash,
                    len: bytes.len() as u64,
                };
            }
        }
        e.hash = e.compute_hash();

        let body = e.encode();
        if self.seg_len > 0 && self.seg_len + 8 + body.len() as u64 > SEGMENT_MAX_BYTES {
            self.file.sync_all()?;
            self.seg_index += 1;
            let dir = stream_dir(&self.root, self.stream);
            self.file = OpenOptions::new()
                .create_new(true)
                .append(true)
                .open(dir.join(format!("seg-{:06}.hslog", self.seg_index)))?;
            fsync_dir(&dir)?;
            self.seg_len = 0;
        }
        self.seg_len += write_frame(&mut self.file, &body)?;
        self.next_seq += 1;
        self.last_hash = e.hash;
        self.last_event_id = Some(e.event_id);
        Ok(e)
    }

    fn store_blob(&self, bytes: &[u8]) -> Result<[u8; 32], LogError> {
        write_blob(&self.root, bytes)
    }
}

/// Content-addressed blob store (gate 6's world service stores artifact
/// content here by hash; same durability discipline as payloads).
pub fn write_blob(root: &Path, bytes: &[u8]) -> Result<[u8; 32], LogError> {
    {
        let hash: [u8; 32] = Sha256::digest(bytes).into();
        let path = blob_path_inner(root, &hash);
        if !path.exists() {
            let dir = path.parent().unwrap().to_path_buf();
            fs::create_dir_all(&dir)?;
            let tmp = dir.join(format!(".tmp-{}", Uuid::new_v4()));
            {
                let mut f = File::create(&tmp)?;
                f.write_all(bytes)?;
                f.sync_all()?;
            }
            fs::rename(&tmp, &path)?;
            fsync_dir(&dir)?;
        }
        Ok(hash)
    }
}

/// Bulk blob write for large content sets (snapshots). Same
/// content-addressed store and durability outcome as [write_blob] - every
/// blob is complete and durable when this returns - but the flush is
/// amortized: blobs are written, then a single syncfs commits the whole
/// batch, instead of one fsync pair per blob.
pub fn write_blobs_bulk(root: &Path, items: &[&[u8]]) -> Result<Vec<[u8; 32]>, LogError> {
    let mut out = Vec::with_capacity(items.len());
    let mut dirs_touched: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
    for bytes in items {
        let hash: [u8; 32] = Sha256::digest(bytes).into();
        let path = blob_path_inner(root, &hash);
        if !path.exists() {
            let dir = path.parent().unwrap().to_path_buf();
            fs::create_dir_all(&dir)?;
            let tmp = dir.join(format!(".tmp-{}", Uuid::new_v4()));
            {
                let mut f = File::create(&tmp)?;
                f.write_all(bytes)?;
            }
            fs::rename(&tmp, &path)?;
            dirs_touched.insert(dir);
        }
        out.push(hash);
    }
    if !dirs_touched.is_empty() {
        // One filesystem-level flush for the batch (Linux). Falls back to
        // per-directory fsync where syncfs is unavailable.
        #[cfg(target_os = "linux")]
        {
            let f = File::open(root)?;
            let fd = std::os::unix::io::AsRawFd::as_raw_fd(&f);
            let rc = unsafe { libc::syncfs(fd) };
            if rc != 0 {
                return Err(LogError::Io(std::io::Error::last_os_error()));
            }
        }
        #[cfg(not(target_os = "linux"))]
        for d in &dirs_touched {
            fsync_dir(d)?;
        }
    }
    Ok(out)
}

/// Read a blob by content hash.
pub fn read_blob(root: &Path, hash: &[u8; 32]) -> Result<Vec<u8>, LogError> {
    Ok(fs::read(blob_path_inner(root, hash))?)
}

fn check_chain(e: &Event, expected_seq: u64, expected_prev: [u8; 32]) -> Result<(), LogError> {
    if e.seq != expected_seq {
        return Err(LogError::Corruption(Corruption {
            seq: expected_seq,
            kind: CorruptionKind::SeqGap {
                expected: expected_seq,
                found: e.seq,
            },
        }));
    }
    if e.prev_hash != expected_prev {
        return Err(LogError::Corruption(Corruption {
            seq: e.seq,
            kind: CorruptionKind::PrevHashMismatch {
                expected: expected_prev,
                found: e.prev_hash,
            },
        }));
    }
    if !e.verify_hash() {
        return Err(LogError::Corruption(Corruption {
            seq: e.seq,
            kind: CorruptionKind::HashMismatch,
        }));
    }
    Ok(())
}

pub struct StreamReader {
    root: PathBuf,
    stream: Uuid,
}

impl StreamReader {
    pub fn open(root: &Path, stream: Uuid) -> Result<Self, LogError> {
        if segment_files(root, stream)?.is_empty() {
            return Err(LogError::StreamMissing(stream));
        }
        Ok(StreamReader {
            root: root.to_path_buf(),
            stream,
        })
    }

    /// All events in chain order. CRC/decode failures surface as corruption;
    /// chain linkage is NOT checked here (use verify_stream for that).
    pub fn events(&self) -> Result<Vec<Event>, LogError> {
        let mut out = vec![];
        for path in segment_files(&self.root, self.stream)? {
            let mut f = File::open(&path)?;
            loop {
                match read_frame(&mut f)? {
                    FrameRead::End | FrameRead::TornTail(_) => break,
                    FrameRead::Corrupt(mut c) => {
                        c.seq = out.len() as u64;
                        return Err(LogError::Corruption(c));
                    }
                    FrameRead::Ok(e, _) => out.push(e),
                }
            }
        }
        Ok(out)
    }

    /// Events up to and including `seq`: the rewind read path.
    pub fn replay_to(&self, seq: u64) -> Result<Vec<Event>, LogError> {
        Ok(self
            .events()?
            .into_iter()
            .take_while(|e| e.seq <= seq)
            .collect())
    }

    pub fn resolve_payload(&self, e: &Event) -> Result<Vec<u8>, LogError> {
        match &e.payload {
            Payload::None => Ok(vec![]),
            Payload::Inline(b) => Ok(b.clone()),
            Payload::BlobRef { hash, .. } => {
                let path = blob_path_inner(&self.root, hash);
                if !path.exists() {
                    return Err(LogError::Corruption(Corruption {
                        seq: e.seq,
                        kind: CorruptionKind::BlobMissing { hash: *hash },
                    }));
                }
                let bytes = fs::read(&path)?;
                let actual: [u8; 32] = Sha256::digest(&bytes).into();
                if actual != *hash {
                    return Err(LogError::Corruption(Corruption {
                        seq: e.seq,
                        kind: CorruptionKind::BlobHashMismatch { hash: *hash },
                    }));
                }
                Ok(bytes)
            }
        }
    }
}

/// Full verification: frame CRCs, decode, hash chain, seq contiguity, and
/// every referenced blob present and matching its content hash.
pub fn verify_stream(root: &Path, stream: Uuid) -> Result<VerifyReport, Corruption> {
    let reader = StreamReader::open(root, stream).map_err(|e| Corruption {
        seq: 0,
        kind: CorruptionKind::Decode(e.to_string()),
    })?;
    let events = reader.events().map_err(|e| match e {
        LogError::Corruption(c) => c,
        other => Corruption {
            seq: 0,
            kind: CorruptionKind::Decode(other.to_string()),
        },
    })?;
    let mut expected_seq = 0u64;
    let mut expected_prev = [0u8; 32];
    for e in &events {
        check_chain(e, expected_seq, expected_prev).map_err(|e| match e {
            LogError::Corruption(c) => c,
            other => Corruption {
                seq: expected_seq,
                kind: CorruptionKind::Decode(other.to_string()),
            },
        })?;
        if let Payload::BlobRef { .. } = &e.payload {
            reader.resolve_payload(e).map_err(|le| match le {
                LogError::Corruption(c) => c,
                other => Corruption {
                    seq: e.seq,
                    kind: CorruptionKind::Decode(other.to_string()),
                },
            })?;
        }
        expected_seq = e.seq + 1;
        expected_prev = e.hash;
    }
    Ok(VerifyReport {
        events: events.len() as u64,
        last_hash: expected_prev,
    })
}

/// Test-support helpers: deliberate log surgery for the corruption proofs.
/// Not used by any production path.
pub mod testing {
    use super::*;
    use std::io::{Seek, SeekFrom};

    pub fn blob_path(root: &Path, hash: &[u8; 32]) -> PathBuf {
        blob_path_inner(root, hash)
    }

    /// TEST-SUPPORT tamper utility (integrity proofs only): flip one byte
    /// inside the encoded body of the event with `seq`. Panics when no
    /// event carries `seq` - a test authoring error, not a runtime path.
    pub fn corrupt_event_byte(root: &Path, stream: Uuid, seq: u64, body_offset: usize) {
        for path in segment_files(root, stream).unwrap() {
            let mut f = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .unwrap();
            let mut frame_start = 0u64;
            loop {
                let mut hdr = [0u8; 8];
                if f.read_exact(&mut hdr).is_err() {
                    break;
                }
                let len = u32::from_le_bytes(hdr[0..4].try_into().unwrap()) as usize;
                let mut body = vec![0u8; len];
                f.read_exact(&mut body).unwrap();
                if let Ok(e) = Event::decode(&body) {
                    if e.seq == seq {
                        let off = body_offset.min(len - 1);
                        f.seek(SeekFrom::Start(frame_start + 8 + off as u64))
                            .unwrap();
                        let mut b = [0u8; 1];
                        f.read_exact(&mut b).unwrap();
                        f.seek(SeekFrom::Start(frame_start + 8 + off as u64))
                            .unwrap();
                        f.write_all(&[b[0] ^ 0xFF]).unwrap();
                        f.sync_all().unwrap();
                        return;
                    }
                }
                frame_start += 8 + len as u64;
            }
        }
        panic!("no event with seq {seq}");
    }

    /// TEST-SUPPORT tamper utility (integrity proofs only): rewrite the
    /// segment files without the event at `seq` (simulates a silently
    /// deleted record).
    pub fn remove_event_from_segment(root: &Path, stream: Uuid, seq: u64) {
        let files = segment_files(root, stream).unwrap();
        let mut kept: Vec<Vec<u8>> = vec![];
        for path in &files {
            let mut f = File::open(path).unwrap();
            loop {
                let mut hdr = [0u8; 8];
                if f.read_exact(&mut hdr).is_err() {
                    break;
                }
                let len = u32::from_le_bytes(hdr[0..4].try_into().unwrap()) as usize;
                let mut body = vec![0u8; len];
                f.read_exact(&mut body).unwrap();
                let e = Event::decode(&body).unwrap();
                if e.seq != seq {
                    let mut frame = hdr.to_vec();
                    frame.extend(body);
                    kept.push(frame);
                }
            }
        }
        for path in &files {
            fs::remove_file(path).unwrap();
        }
        let dir = stream_dir(root, stream);
        let mut out = File::create(dir.join("seg-000000.hslog")).unwrap();
        for frame in kept {
            out.write_all(&frame).unwrap();
        }
        out.sync_all().unwrap();
    }
}
