//! WORLD-PLANE efficacy pins (Eric's order: "coordination genuinely
//! flows through the world - no side channels; mission B cannot obtain
//! A's artifact if the world rejects the proposal; observe returns
//! nothing before validation"). Every observer here is a FRESH
//! `World::open` on the same log root, so visibility can only flow
//! through consequence events on the world stream - never through
//! shared memory between agents.

use hs_world::{Artifact, ArtifactKind, ArtifactStatus, World};

fn artifact(path: &str, content: &[u8]) -> Artifact {
    Artifact {
        artifact_id: uuid::Uuid::new_v4(),
        version: 1,
        kind: ArtifactKind::Note,
        content_hash: sha2::Sha256::digest(content).into(),
        world_path: path.to_string(),
        author_stream: uuid::Uuid::new_v4(),
        parent_version: None,
        status: ArtifactStatus::Proposed,
    }
}

use sha2::Digest;

// w1: a REJECTED proposal (forged content hash) is invisible to any
// later observer - B cannot obtain A's artifact when the world rejects.
#[test]
fn w1_rejected_proposal_is_invisible_to_observers() {
    let dir = tempfile::tempdir().unwrap();
    let world = World::open(dir.path()).unwrap();
    let mut a = artifact("/shared/notes", b"the real content");
    a.content_hash = [0u8; 32]; // forged: does not match the content
    assert!(
        world.propose(a, b"the real content").is_err(),
        "the world must reject the forged proposal"
    );
    let observer = World::open(dir.path()).unwrap();
    assert!(
        observer.observe("/shared/notes").unwrap().is_empty(),
        "a rejected proposal must never become observable"
    );
}

// w2: observe returns NOTHING before validation - an artifact the
// author never submitted does not exist for anyone else - and the
// positive control: once the world validates it, a fresh observer sees
// exactly it (guards against a rig that passes by observing nothing).
#[test]
fn w2_nothing_before_validation_then_exactly_the_artifact() {
    let dir = tempfile::tempdir().unwrap();
    let world = World::open(dir.path()).unwrap();
    let a = artifact("/shared/notes", b"validated bytes");
    let id = a.artifact_id;
    // author holds the artifact, has not proposed it yet
    let early = World::open(dir.path()).unwrap();
    assert!(
        early.observe("/shared/notes").unwrap().is_empty(),
        "unvalidated work is invisible"
    );
    let validated = world.propose(a, b"validated bytes").unwrap();
    assert_eq!(validated.status, ArtifactStatus::Validated);
    let late = World::open(dir.path()).unwrap();
    let seen = late.observe("/shared/notes").unwrap();
    assert_eq!(seen.len(), 1, "exactly the validated artifact");
    assert_eq!(seen[0].artifact_id, id);
    assert_eq!(seen[0].author_stream, validated.author_stream);
}

// w3: the LOG is the coordination medium - an observer opened BEFORE
// the validation sees nothing even while the author world has the
// artifact in its own state; one opened AFTER sees it. No shared-memory
// side channel between world instances.
#[test]
fn w3_visibility_flows_through_the_stream_not_shared_memory() {
    let dir = tempfile::tempdir().unwrap();
    let author = World::open(dir.path()).unwrap();
    let before = World::open(dir.path()).unwrap();
    author
        .propose(artifact("/shared/notes", b"coordinated"), b"coordinated")
        .unwrap();
    assert!(
        before.observe("/shared/notes").unwrap().is_empty(),
        "a stale observer sees nothing new: no shared-memory side channel"
    );
    let after = World::open(dir.path()).unwrap();
    assert_eq!(
        after.observe("/shared/notes").unwrap().len(),
        1,
        "a fresh observer reads the validated consequence off the log"
    );
}

// w4: even a FORGED consequence event naming a still-Proposed artifact
// (written straight onto the stream, bypassing World::propose) stays
// invisible at the observe boundary - validation status, not presence
// on the stream, is the gate.
#[test]
fn w4_forged_proposed_consequence_stays_invisible() {
    let dir = tempfile::tempdir().unwrap();
    let world = World::open(dir.path()).unwrap();
    let forged = artifact("/shared/notes", b"forged consequence");
    let stream = world.world_stream();
    let mut w = hs_log::StreamWriter::resume(dir.path(), stream)
        .unwrap()
        .writer;
    w.append(
        hs_core::EventBuilder::new(hs_core::EventKind::Consequence).payload(
            hs_core::Payload::Inline(serde_json::to_vec(&forged).unwrap()),
        ),
    )
    .unwrap();
    drop(w);
    let observer = World::open(dir.path()).unwrap();
    assert!(
        observer.observe("/shared/notes").unwrap().is_empty(),
        "a consequence naming a Proposed artifact is not observable"
    );
}
