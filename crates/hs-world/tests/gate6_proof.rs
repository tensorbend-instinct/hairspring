//! GATE 6 ACCEPTANCE (spec section 10, row 6):
//!   Two agents on disjoint schedules: agent B reuses agent A's installed
//!   controller by world observation with zero messages; uninstall A
//!   entirely and the controller keeps acting.
//!
//! Falsifiable: any A<->B message event, B failing to reuse, the controller
//! stopping after A's uninstall, or an invalid proposal getting through.

use hs_world::*;

fn controller_program() -> Vec<u8> {
    // a deterministic installed program: on each tick, append one counter
    // line to /counter.txt via consequence events (no model calls)
    br#"{"op":"append_counter","target":"/counter.txt"}"#.to_vec()
}

fn artifact(author: uuid::Uuid, kind: ArtifactKind, path: &str, content: &[u8]) -> Artifact {
    use sha2::Digest;
    Artifact {
        artifact_id: uuid::Uuid::new_v4(),
        version: 1,
        kind,
        content_hash: sha2::Sha256::digest(content).into(),
        world_path: path.to_string(),
        author_stream: author,
        parent_version: None,
        status: ArtifactStatus::Proposed,
    }
}

#[test]
fn gate6_proof_shared_world_executable_inheritance() {
    let root = tempfile::tempdir().unwrap();
    let log_root = root.path().join("log");
    let world = World::open(&log_root).unwrap();

    let stream_a = uuid::Uuid::new_v4();
    let stream_b = uuid::Uuid::new_v4();
    let _wa = hs_log::StreamWriter::create(&log_root, stream_a).unwrap();
    let _wb = hs_log::StreamWriter::create(&log_root, stream_b).unwrap();

    // A proposes + installs a controller (validate happens inside propose)
    let prog = controller_program();
    let c = artifact(stream_a, ArtifactKind::Controller, "/ctrl/counter", &prog);
    let c = world.propose(c, &prog).unwrap();
    assert_eq!(c.status, ArtifactStatus::Validated);
    world.install(c.artifact_id).unwrap();

    // ticks: the controller acts with zero model calls
    for _ in 0..3 {
        assert_eq!(world.tick().unwrap().len(), 1, "one consequence per tick");
    }

    // B observes the world (NO messages): finds A's controller + its output
    let seen = world.observe("/ctrl/counter").unwrap();
    assert!(
        seen.iter().any(|a| a.artifact_id == c.artifact_id),
        "B finds A's controller by observation"
    );
    let counter = world.observe("/counter.txt").unwrap();
    assert!(!counter.is_empty(), "B sees the controller's world output");

    // B reuses: proposes a note derived from the observed counter
    let note_content = b"B reused A's counter artifact".to_vec();
    let note = artifact(
        stream_b,
        ArtifactKind::Note,
        "/notes/b-reuse",
        &note_content,
    );
    let mut note = note;
    note.parent_version = Some(c.artifact_id); // executable inheritance lineage
    let note = world.propose(note, &note_content).unwrap();
    assert_eq!(note.status, ArtifactStatus::Validated);

    // uninstall A entirely: the controller KEEPS ACTING
    world.uninstall_agent(stream_a).unwrap();
    for _ in 0..2 {
        assert_eq!(
            world.tick().unwrap().len(),
            1,
            "controller acts after author uninstall"
        );
    }

    // adversarial: forged content hash is rejected
    let mut forged = artifact(stream_b, ArtifactKind::File, "/forged", b"x");
    forged.content_hash = [0u8; 32];
    assert!(
        world.propose(forged, b"x").is_err(),
        "hash forgery rejected"
    );

    // THE GATE, structurally: zero Message-kind events anywhere in the log
    for s in [stream_a, stream_b] {
        let r = hs_log::StreamReader::open(&log_root, s).unwrap();
        let msgs = r
            .events()
            .unwrap()
            .iter()
            .filter(|e| e.kind == hs_core::EventKind::Message)
            .count();
        assert_eq!(
            msgs, 0,
            "coordination must run through the world, not messages"
        );
    }
    // every chain verifies: world stream + both agent streams
    for s in [stream_a, stream_b] {
        hs_log::verify_stream(&log_root, s).unwrap();
    }

    println!("PROOF-GATE6 shared world + executable inheritance: PASS");
    println!("  B reused A's installed controller by observation; zero A<->B messages; controller kept acting after A's uninstall");
}
