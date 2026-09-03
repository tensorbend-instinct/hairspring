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
}

/// The shared world: artifact registry + installed controllers, all state
/// recorded as Proposal/Consequence events on a world stream in the shared
/// log root. State is rebuilt by replaying that stream on open.
pub struct World {
    log_root: PathBuf,
    world_stream: uuid::Uuid,
    state: Mutex<State>,
}

impl World {
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
            state: Mutex::new(State { artifacts }),
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
