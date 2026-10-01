//! RED: skill index/loader over INSTALLED world skills (Hermes-style
//! progressive disclosure: a compact name+description index in the prompt,
//! full SKILL.md body only on skill.view).

fn mk(log: &std::path::Path, author: uuid::Uuid, path: &str, content: &str) -> uuid::Uuid {
    use sha2::Digest;
    let a = hs_world::Artifact {
        artifact_id: uuid::Uuid::new_v4(),
        version: 1,
        kind: hs_world::ArtifactKind::Skill,
        content_hash: sha2::Sha256::digest(content.as_bytes()).into(),
        world_path: path.into(),
        author_stream: author,
        parent_version: None,
        status: hs_world::ArtifactStatus::Proposed,
    };
    let _ = log;
    let id = a.artifact_id;
    w(log).propose(a, content.as_bytes()).unwrap();
    id
}
fn w(log: &std::path::Path) -> hs_world::World {
    hs_world::World::open(log).unwrap()
}

#[test]
fn index_lists_only_installed_skills_with_frontmatter() {
    let dir = std::env::temp_dir().join(format!("hsskill-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let log = dir.join("log");
    std::fs::create_dir_all(&log).unwrap();
    let author = uuid::Uuid::new_v4();
    hs_log::StreamWriter::create(&log, author).unwrap();

    let good = mk(&log, author, "/skills/git-bisect", "---\nname: git-bisect\ndescription: Find the commit that broke a test\n---\n# Steps\n1. git bisect start\n");
    let _validated_only = mk(&log, author, "/skills/draft", "---\nname: draft\ndescription: not installed\n---\nbody\n");
    let nofm = mk(&log, author, "/skills/nofm", "no frontmatter at all\n");

    let world = w(&log);
    assert!(world.skill_index().is_empty(), "nothing installed yet");
    world.install_skill(good).unwrap();
    world.install_skill(nofm).unwrap();

    let idx = world.skill_index();
    assert_eq!(idx.len(), 1, "skill without valid frontmatter is not indexed: {idx:?}");
    assert_eq!(idx[0].name, "git-bisect");
    assert_eq!(idx[0].description, "Find the commit that broke a test");

    let block = world.skill_index_block();
    assert!(block.contains("git-bisect: Find the commit that broke a test"));
    assert!(!block.contains("# Steps"), "index must not leak the body");
    assert!(!block.contains("draft"));

    let body = world.skill_view("git-bisect").unwrap();
    assert!(body.contains("git bisect start"));
    assert!(world.skill_view("draft").is_err());
    assert!(world.skill_view("../etc/passwd").is_err());
}

#[test]
fn install_skill_rejects_non_skill_and_unvalidated() {
    let dir = std::env::temp_dir().join(format!("hsskill2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let log = dir.join("log");
    std::fs::create_dir_all(&log).unwrap();
    let world = w(&log);
    assert!(world.install_skill(uuid::Uuid::new_v4()).is_err());
}
