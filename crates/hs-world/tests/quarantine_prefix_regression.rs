use hs_world::{Artifact,ArtifactKind,ArtifactStatus,World};
use sha2::Digest;

fn item(author: uuid::Uuid, path: &std::path::Path, content: &[u8]) -> Artifact {
    Artifact { artifact_id: uuid::Uuid::new_v4(), version: 1,
        kind: ArtifactKind::File, content_hash: sha2::Sha256::digest(content).into(),
        world_path: path.display().to_string(), author_stream: author,
        parent_version: None, status: ArtifactStatus::Proposed }
}

#[test]
fn quarantined_stream_cannot_escape_by_prefix_or_symlink() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("sandbox");
    let world = World::open(&root).unwrap();
    let author = uuid::Uuid::new_v4();
    world.quarantine(author);
    let prefix = tmp.path().join("sandbox-elsewhere").join("target");
    assert!(world.propose(item(author,&prefix,b"x"),b"x").is_err());
    assert!(!prefix.exists());
    let outside = tmp.path().join("outside"); std::fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside,root.join("link")).unwrap();
    let via_link = root.join("link/target");
    assert!(world.propose(item(author,&via_link,b"x"),b"x").is_err());
    assert!(!outside.join("target").exists());
    let inside = root.join("normal/target");
    let good=world.propose(item(author,&inside,b"ok"),b"ok").unwrap();
    world.materialize(&good).unwrap();
    assert_eq!(std::fs::read(inside).unwrap(),b"ok");
}

#[test]
fn materialize_rechecks_quarantine_after_validation() {
    let tmp=tempfile::tempdir().unwrap();let root=tmp.path().join("sandbox");
    let world=World::open(&root).unwrap();let author=uuid::Uuid::new_v4();
    let outside=tmp.path().join("external");
    let validated=world.propose(item(author,&outside,b"x"),b"x").unwrap();
    world.quarantine(author);
    assert!(world.materialize(&validated).is_err());
    assert!(!outside.exists());
}
