use hs_core::{EventBuilder, EventKind};
use hs_log::{LogError, StreamWriter};
use std::io::Write;
use uuid::Uuid;

#[test]
fn held_stream_cannot_be_truncated_during_resume() {
    let root = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    let mut writer = StreamWriter::create(root.path(), id).unwrap();
    writer.append(EventBuilder::new(EventKind::ToolCall)).unwrap();
    let path = root.path().join("streams").join(id.to_string()).join("seg-000000.hslog");
    // Simulate a torn tail while the original writer is still active.
    std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(&[1, 2, 3]).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(matches!(StreamWriter::resume(root.path(), id), Err(LogError::StreamHeld(_))));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    drop(writer);
    let recovered = StreamWriter::resume(root.path(), id).unwrap();
    assert_eq!(recovered.truncated_bytes, 3);
}
