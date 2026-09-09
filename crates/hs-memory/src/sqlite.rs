//! SQLite WAL implementation of the memory plane.

use crate::{MemoryError, MemoryRecord, MemoryStore, NewMemoryRecord};
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

pub struct SqliteMemoryStore {
    conn: Mutex<Connection>,
}

impl SqliteMemoryStore {
    pub fn open(path: &Path) -> Result<Self, MemoryError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memory_records (
              id            TEXT PRIMARY KEY,
              agent_id      TEXT NOT NULL,
              mission_id    TEXT,
              kind          TEXT NOT NULL,
              content       TEXT NOT NULL,
              embedding     BLOB,
              importance    REAL NOT NULL DEFAULT 0.5,
              expires_at    INTEGER,
              source_seqs   TEXT NOT NULL,
              created_at    INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS memory_edges (
              from_id TEXT NOT NULL, to_id TEXT NOT NULL,
              rel TEXT NOT NULL,
              PRIMARY KEY (from_id, to_id, rel)
            );",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

impl MemoryStore for SqliteMemoryStore {
    fn put(&self, r: NewMemoryRecord) -> Result<String, MemoryError> {
        let id = uuid::Uuid::new_v4().to_string();
        let seqs = serde_json::to_string(&r.source_seqs).unwrap_or_else(|_| "[]".into());
        let conn = self.conn.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        conn.execute(
            "INSERT INTO memory_records
             (id, agent_id, mission_id, kind, content, embedding, importance, expires_at, source_seqs, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9)",
            params![id, r.agent_id, r.mission_id, r.kind, r.content, r.importance, r.expires_at, seqs, now_ms()],
        )?;
        Ok(id)
    }

    fn top_k(&self, agent_id: &str, k: usize) -> Result<Vec<MemoryRecord>, MemoryError> {
        let conn = self.conn.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut st = conn.prepare(
            "SELECT id, agent_id, mission_id, kind, content, importance, expires_at, source_seqs, created_at
             FROM memory_records
             WHERE agent_id = ?1 AND (expires_at IS NULL OR expires_at > ?2)
             ORDER BY importance DESC, created_at DESC LIMIT ?3",
        )?;
        let rows = st.query_map(params![agent_id, now_ms(), k as i64], |row| {
            let seqs_json: String = row.get(7)?;
            Ok(MemoryRecord {
                id: row.get(0)?,
                agent_id: row.get(1)?,
                mission_id: row.get(2)?,
                kind: row.get(3)?,
                content: row.get(4)?,
                importance: row.get(5)?,
                expires_at: row.get(6)?,
                source_seqs: serde_json::from_str(&seqs_json).unwrap_or_default(),
                created_at: row.get(8)?,
            })
        })?;
        Ok(rows.filter_map(std::result::Result::ok).collect())
    }
}
