//! RED: world-plane reuse bookkeeping (SwarmWorld-fidelity gap 3).
//! Every world.observe that DELIVERS artifacts to an observing stream is a
//! reuse interaction: book it onto the world stream (observer + artifact +
//! version) and expose reuse_count, so culture diffusion is measurable
//! from the log alone (SwarmWorld S2.5: recorded provenance).

use hs_core::EventKind;

fn world_payloads(log: &std::path::Path, kind: EventKind) -> Vec<String> {
    let reader = hs_log::StreamReader::open(log, hs_world::world_stream_id()).unwrap();
    reader
        .events()
        .unwrap()
        .iter()
        .filter(|e| e.kind == kind)
        .filter_map(|e| reader.resolve_payload(e).ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .collect()
}

fn artifact(path: &str, author: uuid::Uuid, content: &[u8]) -> hs_world::Artifact {
    let hash: [u8; 32] = {
        use sha2::Digest;
        sha2::Sha256::digest(content).into()
    };
    hs_world::Artifact {
        artifact_id: uuid::Uuid::new_v4(),
        version: 1,
        kind: hs_world::ArtifactKind::Skill,
        content_hash: hash,
        world_path: path.to_string(),
        author_stream: author,
        parent_version: None,
        status: hs_world::ArtifactStatus::Proposed,
    }
}

#[test]
fn observe_books_reuse_and_counts_it() {
    let dir = std::env::temp_dir().join(format!("hsreuse-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let log = dir.join("log");
    std::fs::create_dir_all(&log).unwrap();

    let world = hs_world::World::open(&log).unwrap();
    let author = uuid::Uuid::new_v4();
    let observer = uuid::Uuid::new_v4();
    hs_log::StreamWriter::create(&log, author).unwrap();
    hs_log::StreamWriter::create(&log, observer).unwrap();

    let content = b"the reusable move";
    let a = artifact("/skills/move", author, content);
    let id = a.artifact_id;
    world.propose(a, content).unwrap();

    // A plain administrative observe books nothing ...
    let before = world_payloads(&log, EventKind::Observation)
        .iter()
        .filter(|o| o.contains("\"reuse\""))
        .count();
    assert_eq!(world.reuse_count(id, 1), 0, "no reuse before any observe");

    // ... an attributed observe books exactly one reuse event per
    // delivered artifact, naming the OBSERVING stream.
    let got = world.observe_as(observer, "/skills/move").unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(world.reuse_count(id, 1), 1);
    let got = world.observe_as(observer, "/skills/move").unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(world.reuse_count(id, 1), 2);

    let obs = world_payloads(&log, EventKind::Observation);
    let reuses: Vec<&String> = obs.iter().filter(|o| o.contains("\"reuse\"")).collect();
    assert_eq!(
        reuses.len(),
        before + 2,
        "each attributed observe books one reuse event: {obs:?}"
    );
    assert!(
        reuses
            .iter()
            .all(|o| o.contains(&observer.to_string()) && o.contains(&id.to_string())),
        "reuse events must name observer stream + artifact id: {reuses:?}"
    );

    // Reuse state survives a fresh World handle (rebuilt from the stream).
    let world2 = hs_world::World::open(&log).unwrap();
    assert_eq!(world2.reuse_count(id, 1), 2, "reuse count lost on reopen");

    let _ = std::fs::remove_dir_all(&dir);
}
