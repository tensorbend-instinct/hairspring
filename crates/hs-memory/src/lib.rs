//! D3 typed memory plane (`TencentDB` trinity, replicated for one box).
//! One embedded store, SQLite in WAL mode, behind a trait so
//! Postgres+pgvector+AGE can replace it (W1: the trait boundary is the
//! contract). Writes are append-only and immutable; provenance is
//! mandatory: every record points at the log seqs it was distilled from.
//! Embeddings are null until the embedding job lands; retrieval is
//! importance-then-recency top-k until sqlite-vec is wired in.

pub mod extract;
pub mod sqlite;

#[derive(Debug, Clone)]
pub struct NewMemoryRecord {
    pub agent_id: String,
    pub mission_id: Option<String>,
    pub kind: String, // episodic|semantic|procedural|preference
    pub content: String,
    pub importance: f64,
    pub expires_at: Option<i64>,
    pub source_seqs: Vec<u64>,
}

#[derive(Debug, Clone)]
pub struct MemoryRecord {
    pub id: String,
    pub agent_id: String,
    pub mission_id: Option<String>,
    pub kind: String,
    pub content: String,
    pub importance: f64,
    pub expires_at: Option<i64>,
    pub source_seqs: Vec<u64>,
    pub created_at: i64,
}

#[derive(Debug)]
pub enum MemoryError {
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
}
impl From<rusqlite::Error> for MemoryError {
    fn from(e: rusqlite::Error) -> Self {
        MemoryError::Sqlite(e)
    }
}
impl From<std::io::Error> for MemoryError {
    fn from(e: std::io::Error) -> Self {
        MemoryError::Io(e)
    }
}
impl std::fmt::Display for MemoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryError::Sqlite(e) => write!(f, "sqlite: {e}"),
            MemoryError::Io(e) => write!(f, "io: {e}"),
        }
    }
}
impl std::error::Error for MemoryError {}

/// The store contract. Retrieval happens at assembly time (D1), never by
/// pasting memory into prompts outside the assembler.
pub trait MemoryStore: Send + Sync {
    /// Append-only write; returns the new record id.
    fn put(&self, r: NewMemoryRecord) -> Result<String, MemoryError>;
    /// Top-k by importance, then recency. Cross-mission by design (the
    /// point of the plane); mission scoping is the caller's filter.
    fn top_k(&self, agent_id: &str, k: usize) -> Result<Vec<MemoryRecord>, MemoryError>;
}
