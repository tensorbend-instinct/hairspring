//! RED (hostile review, 2026-09-09): `SqliteMemoryStore::top_k` silently
//! DROPS rows whose decode fails (`filter_map(Result::ok)`), turning store
//! corruption into silently missing memory. The read path must surface a
//! decode failure, not hide it.
//!
//! Falsifier: corrupt one stored row (SQLite's dynamic typing lets a TEXT
//! value land in the REAL importance column); `top_k` must return Err, not
//! Ok-with-the-row-missing.

use hs_memory::sqlite::SqliteMemoryStore;
use hs_memory::{MemoryStore, NewMemoryRecord};

#[test]
fn top_k_surfaces_row_decode_errors_instead_of_dropping_rows() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("mem.db");
    let store = SqliteMemoryStore::open(&db).unwrap();
    store
        .put(NewMemoryRecord {
            agent_id: "a".into(),
            mission_id: None,
            kind: "episodic".into(),
            content: "c".into(),
            importance: 0.6,
            expires_at: None,
            source_seqs: vec![1],
        })
        .unwrap();
    // Corrupt the row through a second connection: a TEXT value in the
    // REAL importance column fails the f64 decode.
    let raw = rusqlite::Connection::open(&db).unwrap();
    raw.execute("UPDATE memory_records SET importance = 'garbage'", [])
        .unwrap();
    drop(raw);
    assert!(
        store.top_k("a", 10).is_err(),
        "corrupt row was silently dropped instead of surfaced as an error"
    );
}
