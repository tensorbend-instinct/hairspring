//! HAIRSPRING gate 6 - shared world service (spec: World service + artifact
//! schema; design docs/gate6-design.md). Proposal-consequence separation:
//! agents write proposals; the world service alone validates and writes
//! consequences.

use std::path::{Path, PathBuf};

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

/// The shared world: artifact registry + installed controllers, all state
/// recorded as events on a world stream in the shared log root.
pub struct World {
    #[allow(dead_code)]
    log_root: PathBuf,
    #[allow(dead_code)]
    world_stream: uuid::Uuid,
}

impl World {
    pub fn open(_log_root: &Path) -> Result<Self, WorldError> {
        unimplemented!("gate 6 red")
    }

    /// An agent proposes an artifact. The world service validates (schema,
    /// content hash, legal status transition) and writes the consequence.
    pub fn propose(&self, _artifact: Artifact, _content: &[u8]) -> Result<Artifact, WorldError> {
        unimplemented!("gate 6 red")
    }

    /// Install a validated controller artifact: it starts acting on ticks.
    pub fn install(&self, _artifact_id: uuid::Uuid) -> Result<(), WorldError> {
        unimplemented!("gate 6 red")
    }

    /// Retire every artifact authored by this stream (uninstall the agent).
    pub fn uninstall_agent(&self, _stream: uuid::Uuid) -> Result<u32, WorldError> {
        unimplemented!("gate 6 red")
    }

    /// Zero-message coordination: read the current validated world state at
    /// a path. This is how agent B finds agent A's work without a message.
    pub fn observe(&self, _world_path: &str) -> Result<Vec<Artifact>, WorldError> {
        unimplemented!("gate 6 red")
    }

    /// Run one world tick: installed controllers act (without any model
    /// call). Returns consequence event ids.
    pub fn tick(&self) -> Result<Vec<uuid::Uuid>, WorldError> {
        unimplemented!("gate 6 red")
    }
}
