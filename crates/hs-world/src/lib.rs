//! HAIRSPRING gate 6 - shared world service (spec: World service + artifact
//! schema; design docs/gate6-design.md). Proposal-consequence separation
//! [PROVEN in SwarmWorld]: agents write Proposal events; the world service
//! alone validates and writes Consequence events. The agent's description
//! of value is never the measurement of value.
//!
//! Executable inheritance: an installed controller is world property. It
//! keeps acting on ticks (runs_without_model_call: true) even after its
//! author stream is uninstalled.

use hs_core::{EventBuilder, EventKind, Payload};
use hs_log::{StreamReader, StreamWriter};
use sha2::Digest;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactKind {
    File,
    Program,
    Controller,
    Note,
    Skill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactStatus {
    Proposed,
    Validated,
    Installed,
    Retired,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Artifact {
    pub artifact_id: uuid::Uuid,
    pub version: u32,
    pub kind: ArtifactKind,
    pub content_hash: [u8; 32],
    pub world_path: String,
    pub author_stream: uuid::Uuid,
    pub parent_version: Option<uuid::Uuid>,
    pub status: ArtifactStatus,
}

#[derive(Debug)]
pub enum WorldError {
    Rejected(String),
    Log(hs_log::LogError),
}

impl From<hs_log::LogError> for WorldError {
    fn from(e: hs_log::LogError) -> Self {
        Self::Log(e)
    }
}

/// Stable stream id for the world service's own stream (uuid v5, DNS ns).
fn world_stream_id() -> uuid::Uuid {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, b"hairspring.world")
}

struct State {
    artifacts: HashMap<(uuid::Uuid, u32), Artifact>,
    /// Streams currently in self-modification quarantine (spec fig 5: assay
    /// forks). Runtime session state - forks are short-lived and in-process,
    /// so this set is intentionally not part of the replayed artifact state.
    quarantined: std::collections::HashSet<uuid::Uuid>,
}

/// The shared world: artifact registry + installed controllers, all state
/// recorded as Proposal/Consequence events on a world stream in the shared
/// log root. State is rebuilt by replaying that stream on open.
pub struct World {
    log_root: PathBuf,
    world_stream: uuid::Uuid,
    state: Mutex<State>,
}

/// Effects a policy layer can attempt to cause. External effects (sends,
/// spend, writes outside the sandbox) are exactly what quarantine exists to
/// deny (spec fig 5: "no sends, no spend, no writes outside the sandbox").
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Effect {
    SendMessage,
    Spend,
    WriteOutsideSandbox,
    /// Work confined to the fork's own sandbox: always allowed.
    SandboxWrite,
}
impl Effect {
    pub fn is_external(self) -> bool {
        !matches!(self, Effect::SandboxWrite)
    }
}

impl World {
    /// Mark a stream as a quarantined self-modification fork.
    pub fn quarantine(&self, stream: uuid::Uuid) {
        self.state.lock().unwrap().quarantined.insert(stream);
    }
    /// Lift quarantine (promotion or rewind ends the fork's session).
    pub fn lift_quarantine(&self, stream: uuid::Uuid) {
        self.state.lock().unwrap().quarantined.remove(&stream);
    }
    /// The world service is the single authority on side effects: a
    /// quarantined fork may NOT cause external effects (spec fig 5). The
    /// rejection is stream-scoped, not blanket: once the fork's lineage is
    /// promoted and quarantine lifts, the same effect class is authorized.
    pub fn authorize_effect(&self, stream: uuid::Uuid, effect: Effect) -> Result<(), WorldError> {
        if effect.is_external() && self.state.lock().unwrap().quarantined.contains(&stream) {
            return Err(WorldError::Rejected(format!(
                "quarantined stream {stream} may not cause external effect {effect:?}"
            )));
        }
        Ok(())
    }
    /// The canonical world stream id (proofs and snapshot verification read it).
    pub fn world_stream(&self) -> uuid::Uuid {
        self.world_stream
    }

    pub fn log_root(&self) -> &std::path::Path {
        &self.log_root
    }

    pub fn open(log_root: &Path) -> Result<Self, WorldError> {
        let stream = world_stream_id();
        if !log_root.join("streams").join(stream.to_string()).exists() {
            StreamWriter::create(log_root, stream)?;
        }
        let reader = StreamReader::open(log_root, stream)?;
        let mut artifacts = HashMap::new();
        for e in reader.events()? {
            if e.kind != EventKind::Consequence {
                continue;
            }
            let bytes = reader.resolve_payload(&e)?;
            let a: Artifact = serde_json::from_slice(&bytes)
                .map_err(|e| WorldError::Rejected(format!("bad consequence payload: {e}")))?;
            artifacts.insert((a.artifact_id, a.version), a);
        }
        Ok(Self {
            log_root: log_root.to_path_buf(),
            world_stream: stream,
            state: Mutex::new(State {
                artifacts,
                quarantined: std::collections::HashSet::new(),
            }),
        })
    }

    fn consequence(&self, a: &Artifact) -> Result<(), WorldError> {
        let mut w = StreamWriter::resume(&self.log_root, self.world_stream)?.writer;
        w.append(
            EventBuilder::new(EventKind::Consequence)
                .payload(Payload::Inline(serde_json::to_vec(a).unwrap())),
        )?;
        self.state
            .lock()
            .unwrap()
            .artifacts
            .insert((a.artifact_id, a.version), a.clone());
        Ok(())
    }

    /// An agent proposes an artifact (recorded as a Proposal event); the
    /// world service validates (schema, content hash, legal transition) and
    /// writes the consequence (Validated) or rejects.
    pub fn propose(&self, mut artifact: Artifact, content: &[u8]) -> Result<Artifact, WorldError> {
        // the proposal itself goes on the world stream: agents never write
        // consequences, but proposals are theirs
        let mut w = StreamWriter::resume(&self.log_root, self.world_stream)?.writer;
        w.append(
            EventBuilder::new(EventKind::Proposal)
                .payload(Payload::Inline(serde_json::to_vec(&artifact).unwrap())),
        )?;

        if artifact.status != ArtifactStatus::Proposed {
            return Err(WorldError::Rejected(
                "new proposals must enter at status proposed".into(),
            ));
        }
        if !artifact.world_path.starts_with('/') {
            return Err(WorldError::Rejected("world_path must be absolute".into()));
        }
        let hash: [u8; 32] = sha2::Sha256::digest(content).into();
        if hash != artifact.content_hash {
            return Err(WorldError::Rejected(
                "content hash mismatch: proposal forged or corrupt".into(),
            ));
        }
        if self
            .state
            .lock()
            .unwrap()
            .artifacts
            .contains_key(&(artifact.artifact_id, artifact.version))
        {
            return Err(WorldError::Rejected(
                "artifact version already exists".into(),
            ));
        }
        // content-addressed storage: the hash IS the address
        hs_log::write_blob(&self.log_root, content)?;
        artifact.status = ArtifactStatus::Validated;
        self.consequence(&artifact)?;
        Ok(artifact)
    }

    /// Install a validated controller/program artifact: it starts acting
    /// on world ticks, without any model call.
    pub fn install(&self, artifact_id: uuid::Uuid) -> Result<(), WorldError> {
        let mut a = self.latest(artifact_id)?;
        if a.status != ArtifactStatus::Validated {
            return Err(WorldError::Rejected(format!(
                "only validated artifacts install (found {:?})",
                a.status
            )));
        }
        if !matches!(a.kind, ArtifactKind::Controller | ArtifactKind::Program) {
            return Err(WorldError::Rejected(
                "only controllers/programs install".into(),
            ));
        }
        a.status = ArtifactStatus::Installed;
        self.consequence(&a)
    }

    /// Retire every non-installed artifact authored by this stream. An
    /// INSTALLED controller is world property: uninstalling its author does
    /// not stop it (spec's executable-inheritance assay shape).
    pub fn uninstall_agent(&self, stream: uuid::Uuid) -> Result<u32, WorldError> {
        let targets: Vec<Artifact> = self
            .state
            .lock()
            .unwrap()
            .artifacts
            .values()
            .filter(|a| {
                a.author_stream == stream
                    && a.status != ArtifactStatus::Installed
                    && a.status != ArtifactStatus::Retired
            })
            .cloned()
            .collect();
        let n = targets.len() as u32;
        for mut a in targets {
            a.status = ArtifactStatus::Retired;
            self.consequence(&a)?;
        }
        Ok(n)
    }

    /// Zero-message coordination: read the current live (validated or
    /// installed) artifacts at a world path. This is how agent B finds
    /// agent A's work without a message.
    pub fn observe(&self, world_path: &str) -> Result<Vec<Artifact>, WorldError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .artifacts
            .values()
            .filter(|a| {
                a.world_path == world_path
                    && matches!(
                        a.status,
                        ArtifactStatus::Validated | ArtifactStatus::Installed
                    )
            })
            .cloned()
            .collect())
    }

    /// Run one world tick: every installed controller acts once, purely
    /// mechanically (no model call). One consequence event id per action.
    pub fn tick(&self) -> Result<Vec<uuid::Uuid>, WorldError> {
        let controllers: Vec<Artifact> = self
            .state
            .lock()
            .unwrap()
            .artifacts
            .values()
            .filter(|a| a.status == ArtifactStatus::Installed && a.kind == ArtifactKind::Controller)
            .cloned()
            .collect();
        let mut ids = Vec::new();
        for c in controllers {
            // controller content = its installed artifact's program; at this
            // gate programs are declarative: {"op":"append_counter","target":p}
            let program = self.content_of(&c)?;
            let op: serde_json::Value = serde_json::from_slice(&program)
                .map_err(|e| WorldError::Rejected(format!("controller program invalid: {e}")))?;
            match op["op"].as_str() {
                Some("append_counter") => {
                    let target = op["target"].as_str().unwrap_or("/counter.txt");
                    let n = self.observe(target)?.len() as u64 + 1;
                    let body = format!("tick {n}");
                    let a = Artifact {
                        artifact_id: uuid::Uuid::new_v4(),
                        version: n as u32,
                        kind: ArtifactKind::File,
                        content_hash: sha2::Sha256::digest(body.as_bytes()).into(),
                        world_path: target.to_string(),
                        author_stream: self.world_stream, // consequence, not agent say-so
                        parent_version: Some(c.artifact_id),
                        status: ArtifactStatus::Validated,
                    };
                    self.consequence(&a)?;
                    ids.push(a.artifact_id);
                }
                other => {
                    return Err(WorldError::Rejected(format!(
                        "unknown controller op {other:?}"
                    )))
                }
            }
        }
        Ok(ids)
    }

    fn latest(&self, artifact_id: uuid::Uuid) -> Result<Artifact, WorldError> {
        self.state
            .lock()
            .unwrap()
            .artifacts
            .values()
            .filter(|a| a.artifact_id == artifact_id)
            .max_by_key(|a| a.version)
            .cloned()
            .ok_or_else(|| WorldError::Rejected("unknown artifact".into()))
    }

    fn content_of(&self, a: &Artifact) -> Result<Vec<u8>, WorldError> {
        // content-addressed: blobs live in the log's blob store by hash
        hs_log::read_blob(&self.log_root, &a.content_hash).map_err(WorldError::Log)
    }
}

// ---------------------------------------------------------------------
// Recovery tier B (spec v5 "Recovery tiers"): snapshot restore. The
// snapshot is content-addressed in the same blob store as the log: every
// file is a blob, the manifest is a blob, and the manifest hash IS the
// snapshot_ref. Restore re-walks the rehydrated tree and recomputes the
// manifest hash - a byte off anywhere fails the restore, never silently.

/// The full manifest: every directory (empty ones too - spec: "filesystem
/// state back") plus every file with its content hash.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SnapshotManifest {
    pub dirs: Vec<String>,
    pub files: Vec<SnapshotEntry>,
    /// Symlinks recorded as links (target string), never followed: real
    /// mission trees (venvs) are full of them, including dangling ones.
    pub symlinks: Vec<SymlinkEntry>,
}

/// A symlink: relative path and its verbatim target.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SymlinkEntry {
    pub path: String,
    pub target: String,
}

/// One manifest entry: a file, its content hash, and its length.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SnapshotEntry {
    pub path: String,
    pub hash: String,
    pub len: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SnapshotReport {
    pub snapshot_id: String,
    pub files: u64,
    pub bytes: u64,
    /// Measured, never rounded (spec: "cold measured and reported, not
    /// rounded down").
    pub took_ms: u128,
}

fn hex32(h: &[u8; 32]) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_hex32(s: &str) -> Result<[u8; 32], WorldError> {
    let b = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16))
        .collect::<Result<Vec<u8>, _>>()
        .map_err(|e| WorldError::Rejected(format!("bad snapshot id: {e}")))?;
    b.try_into()
        .map_err(|_| WorldError::Rejected("snapshot id must be 32 bytes hex".into()))
}

/// Deterministic full-tree walk: sorted relative dir paths + file bytes.
/// (dirs, symlinks, files). symlink_metadata: links are recorded, never
/// followed - a dangling link must not kill the snapshot.
fn walk_tree(
    root: &Path,
) -> Result<(Vec<String>, Vec<SymlinkEntry>, Vec<(String, Vec<u8>)>), WorldError> {
    let mut dirs: Vec<String> = Vec::new();
    let mut links: Vec<SymlinkEntry> = Vec::new();
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut names: Vec<_> = std::fs::read_dir(&dir)
            .map_err(|e| WorldError::Rejected(format!("snapshot walk {}: {e}", dir.display())))?
            .filter_map(|e| e.ok())
            .map(|e| e.file_name())
            .collect();
        names.sort();
        for name in names {
            let p = dir.join(&name);
            let rel = p
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            let md = std::fs::symlink_metadata(&p)
                .map_err(|e| WorldError::Rejected(format!("stat {}: {e}", p.display())))?;
            if md.file_type().is_symlink() {
                let target = std::fs::read_link(&p)
                    .map_err(|e| WorldError::Rejected(format!("readlink {}: {e}", p.display())))?;
                links.push(SymlinkEntry {
                    path: rel,
                    target: target.to_string_lossy().into_owned(),
                });
            } else if md.is_dir() {
                dirs.push(rel);
                stack.push(p);
            } else if md.is_file() {
                let data = std::fs::read(&p)
                    .map_err(|e| WorldError::Rejected(format!("read {}: {e}", p.display())))?;
                out.push((rel, data));
            }
            // sockets/fifos/devices have no place in a mission snapshot and
            // cannot be rehydrated meaningfully; skipped deliberately.
        }
    }
    dirs.sort();
    links.sort_by(|a, b| a.path.cmp(&b.path));
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok((dirs, links, out))
}

fn manifest_bytes(m: &SnapshotManifest) -> Vec<u8> {
    // Canonical: the walk is path-sorted and serde_json emits struct fields
    // in declaration order, so the encoding is deterministic.
    serde_json::to_vec(m).unwrap()
}

impl World {
    /// Capture the full tree (source, .git, venv, build artifacts) into the
    /// content-addressed store and record a SnapshotRef event naming the
    /// manifest hash. Dedup is free: identical files hash to the same blob.
    pub fn snapshot(&self, ws: &Path) -> Result<SnapshotReport, WorldError> {
        let t0 = std::time::Instant::now();
        let (dirs, symlinks, tree) = walk_tree(ws)?;
        let mut manifest_files: Vec<SnapshotEntry> = Vec::with_capacity(tree.len());
        let mut bytes = 0u64;
        // Bulk-write in ~256 MiB batches: one flush per batch instead of one
        // fsync pair per blob (47k-file trees went from ~90 s to seconds).
        const BATCH: usize = 256 * 1024 * 1024;
        let mut i = 0;
        while i < tree.len() {
            let mut j = i;
            let mut batch_bytes = 0usize;
            while j < tree.len() && batch_bytes + tree[j].1.len() <= BATCH {
                batch_bytes += tree[j].1.len();
                j += 1;
            }
            if j == i {
                j = i + 1; // single file larger than the batch
            }
            let refs: Vec<&[u8]> = tree[i..j].iter().map(|(_, d)| d.as_slice()).collect();
            let hashes = hs_log::write_blobs_bulk(&self.log_root, &refs)?;
            for (k, hash) in hashes.iter().enumerate() {
                let (rel, data) = &tree[i + k];
                bytes += data.len() as u64;
                manifest_files.push(SnapshotEntry {
                    path: rel.clone(),
                    hash: hex32(hash),
                    len: data.len() as u64,
                });
            }
            i = j;
        }
        let manifest = SnapshotManifest { dirs, files: manifest_files, symlinks };
        let manifest_hash = hs_log::write_blob(&self.log_root, &manifest_bytes(&manifest))?;
        let snapshot_id = hex32(&manifest_hash);
        let rep = SnapshotReport {
            snapshot_id: snapshot_id.clone(),
            files: manifest.files.len() as u64,
            bytes,
            took_ms: t0.elapsed().as_millis(),
        };
        let mut w = StreamWriter::resume(&self.log_root, self.world_stream)?.writer;
        w.append(
            EventBuilder::new(EventKind::SnapshotRef)
                .payload(Payload::Inline(serde_json::to_vec(&rep).unwrap())),
        )?;
        Ok(rep)
    }

    /// Rehydrate a snapshot into dest and PROVE it: the tree is re-walked
    /// and the manifest hash recomputed; any mismatch fails the restore.
    pub fn restore(&self, snapshot_id: &str, dest: &Path) -> Result<SnapshotReport, WorldError> {
        let t0 = std::time::Instant::now();
        let manifest_hash = parse_hex32(snapshot_id)?;
        let mbytes = hs_log::read_blob(&self.log_root, &manifest_hash).map_err(|_| {
            WorldError::Rejected(format!("unknown snapshot id {snapshot_id}"))
        })?;
        let manifest: SnapshotManifest = serde_json::from_slice(&mbytes)
            .map_err(|e| WorldError::Rejected(format!("corrupt manifest: {e}")))?;
        std::fs::create_dir_all(dest)
            .map_err(|e| WorldError::Rejected(format!("mkdir {}: {e}", dest.display())))?;
        for l in &manifest.symlinks {
            let lp = dest.join(&l.path);
            if !lp.starts_with(dest) {
                return Err(WorldError::Rejected(format!("manifest link escapes dest: {}", l.path)));
            }
            if let Some(parent) = lp.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| WorldError::Rejected(format!("mkdir {}: {e}", parent.display())))?;
            }
            std::os::unix::fs::symlink(&l.target, &lp)
                .map_err(|e| WorldError::Rejected(format!("symlink {}: {e}", lp.display())))?;
        }
        for d in &manifest.dirs {
            let dp = dest.join(d);
            if !dp.starts_with(dest) {
                return Err(WorldError::Rejected(format!("manifest dir escapes dest: {d}")));
            }
            std::fs::create_dir_all(&dp)
                .map_err(|e| WorldError::Rejected(format!("mkdir {}: {e}", dp.display())))?;
        }
        let mut bytes = 0u64;
        for e in &manifest.files {
            let hash = parse_hex32(&e.hash)?;
            let data = hs_log::read_blob(&self.log_root, &hash)
                .map_err(|_| WorldError::Rejected(format!("missing blob {} for {}", e.hash, e.path)))?;
            if data.len() as u64 != e.len {
                return Err(WorldError::Rejected(format!("length mismatch on {}", e.path)));
            }
            let dest_p = dest.join(&e.path);
            // path-escape guard: a forged manifest must not write outside dest
            if !dest_p.starts_with(dest) {
                return Err(WorldError::Rejected(format!("manifest path escapes dest: {}", e.path)));
            }
            if let Some(parent) = dest_p.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e2| WorldError::Rejected(format!("mkdir {}: {e2}", parent.display())))?;
            }
            std::fs::write(&dest_p, &data)
                .map_err(|e2| WorldError::Rejected(format!("write {}: {e2}", dest_p.display())))?;
            bytes += data.len() as u64;
        }
        // Verify: rebuild the manifest from the rehydrated tree.
        let (rdirs, rlinks, tree) = walk_tree(dest)?;
        let rfiles: Vec<SnapshotEntry> = tree
            .iter()
            .map(|(rel, data)| {
                let h: [u8; 32] = sha2::Sha256::digest(data).into();
                SnapshotEntry {
                    path: rel.clone(),
                    hash: hex32(&h),
                    len: data.len() as u64,
                }
            })
            .collect();
        let rebuilt = SnapshotManifest { dirs: rdirs, files: rfiles, symlinks: rlinks };
        let rebuilt_bytes = manifest_bytes(&rebuilt);
        let rebuilt_hash: [u8; 32] = sha2::Sha256::digest(&rebuilt_bytes).into();
        if rebuilt_hash != manifest_hash {
            return Err(WorldError::Rejected(
                "restore verification failed: rehydrated tree does not match the snapshot manifest".into(),
            ));
        }
        Ok(SnapshotReport {
            snapshot_id: snapshot_id.to_string(),
            files: manifest.files.len() as u64,
            bytes,
            took_ms: t0.elapsed().as_millis(),
        })
    }

    /// Restore a snapshot over a LIVE tree (gate-8 async verifier: banking
    /// and vetoes both return the ws to the audited state). The snapshot
    /// is rehydrated into a sibling staging dir and hash-verified THERE
    /// first - a failed restore leaves the live tree untouched - then the
    /// verified tree is swapped into place and the old tree removed.
    pub fn restore_replace(&self, snapshot_id: &str, ws: &Path) -> Result<SnapshotReport, WorldError> {
        let parent = ws
            .parent()
            .ok_or_else(|| WorldError::Rejected(format!("ws {} has no parent", ws.display())))?;
        let stage = parent.join(format!(".hsstage-{}", std::process::id()));
        let old = parent.join(format!(".hsold-{}", std::process::id()));
        if stage.exists() {
            std::fs::remove_dir_all(&stage)
                .map_err(|e| WorldError::Rejected(format!("clear stage: {e}")))?;
        }
        if old.exists() {
            std::fs::remove_dir_all(&old)
                .map_err(|e| WorldError::Rejected(format!("clear old: {e}")))?;
        }
        let rep = self.restore(snapshot_id, &stage)?;
        std::fs::rename(ws, &old)
            .map_err(|e| WorldError::Rejected(format!("park live tree: {e}")))?;
        if let Err(e) = std::fs::rename(&stage, ws) {
            // roll back: the verified tree could not move into place
            let _ = std::fs::rename(&old, ws);
            return Err(WorldError::Rejected(format!("swap verified tree in: {e}")));
        }
        std::fs::remove_dir_all(&old)
            .map_err(|e| WorldError::Rejected(format!("remove old tree: {e}")))?;
        Ok(rep)
    }
}

