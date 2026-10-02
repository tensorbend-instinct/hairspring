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
    world.install_skill_gated(good, &pass()).unwrap();
    world.install_skill_gated(nofm, &pass()).unwrap();

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
    assert!(world.install_skill_gated(uuid::Uuid::new_v4(), &pass()).is_err());
}

fn pass() -> hs_world::SkillGateEvidence {
    hs_world::SkillGateEvidence {
        verifier: "test".into(),
        heldout_with_skill_passed: true,
        heldout_baseline_passed: false,
        control_with_skill_passed: true,
        control_baseline_passed: true,
    }
}

#[test]
fn gate_refuses_failing_heldout_or_control_regression_and_books_evidence() {
    let dir = std::env::temp_dir().join(format!("hsskill3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let log = dir.join("log");
    std::fs::create_dir_all(&log).unwrap();
    let author = uuid::Uuid::new_v4();
    hs_log::StreamWriter::create(&log, author).unwrap();
    let id = mk(&log, author, "/skills/s", "---\nname: s\ndescription: d\n---\nbody\n");
    let world = w(&log);

    let mut fail_heldout = pass();
    fail_heldout.heldout_with_skill_passed = false;
    assert!(world.install_skill_gated(id, &fail_heldout).is_err());

    let mut regress = pass();
    regress.control_with_skill_passed = false; // control passed without the skill, fails with it
    assert!(world.install_skill_gated(id, &regress).is_err());
    assert!(world.skill_index().is_empty(), "refused skill must not be indexed");

    // control that already failed without the skill is not a regression
    let mut ok = pass();
    ok.control_baseline_passed = false;
    ok.control_with_skill_passed = false;
    world.install_skill_gated(id, &ok).unwrap();
    assert_eq!(world.skill_index().len(), 1);

    let reader = hs_log::StreamReader::open(&log, hs_world::world_stream_id()).unwrap();
    let gates: Vec<String> = reader.events().unwrap().iter()
        .filter(|e| e.kind == hs_core::EventKind::Observation)
        .filter_map(|e| reader.resolve_payload(e).ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .filter(|p| p.contains("skill_gate")).collect();
    assert_eq!(gates.len(), 3, "every gate decision is booked: {gates:?}");
    assert!(gates.iter().filter(|g| g.contains("\"admitted\":true")).count() == 1);
}

#[test]
fn usage_ledger_counts_uses_across_reopen_and_archive_hides_but_keeps() {
    let dir = std::env::temp_dir().join(format!("hsskill4-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let log = dir.join("log");
    std::fs::create_dir_all(&log).unwrap();
    let author = uuid::Uuid::new_v4();
    hs_log::StreamWriter::create(&log, author).unwrap();
    let id = mk(&log, author, "/skills/u", "---\nname: u\ndescription: d\n---\nbody\n");
    let world = w(&log);
    world.install_skill_gated(id, &pass()).unwrap();
    assert_eq!(world.skill_use_count(id), 0);
    world.record_skill_use("u", author).unwrap();
    world.record_skill_use("u", author).unwrap();
    assert!(world.record_skill_use("nope", author).is_err());
    assert_eq!(w(&log).skill_use_count(id), 2, "ledger must survive reopen");

    world.archive_skill(id).unwrap();
    assert!(world.skill_index().is_empty());
    assert!(world.skill_view("u").is_err());
    assert_eq!(w(&log).skill_use_count(id), 2, "history kept after archive");
    assert!(world.archive_skill(id).is_err(), "only installed skills archive");
}

#[test]
fn evaluated_gate_runs_the_verifiers_itself() {
    use hs_world::Trial;
    let dir = std::env::temp_dir().join(format!("hsskill4-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let log = dir.join("log");
    std::fs::create_dir_all(&log).unwrap();
    let author = uuid::Uuid::new_v4();
    hs_log::StreamWriter::create(&log, author).unwrap();
    let work = dir.join("work");
    std::fs::create_dir_all(&work).unwrap();

    // held-out verifier: passes only when the skill tells the agent the magic word
    let heldout = Trial::new(r#"test -n "$HS_SKILL_PATH" && grep -q magic-word "$HS_SKILL_PATH""#, &work);
    // control verifier: passes with or without a skill unless the skill says BREAK
    let control = Trial::new(r#"test -z "$HS_SKILL_PATH" || ! grep -q BREAK "$HS_SKILL_PATH""#, &work);

    let helpful = mk(&log, author, "/skills/helpful", "---\nname: helpful\ndescription: knows the magic-word\n---\nmagic-word\n");
    let ev = w(&log).install_skill_evaluated(helpful, "cmd", &heldout, &control).unwrap();
    assert!(ev.heldout_with_skill_passed && !ev.heldout_baseline_passed, "measured, not supplied: {ev:?}");
    assert!(ev.control_with_skill_passed && ev.control_baseline_passed);
    assert_eq!(w(&log).skill_index().len(), 1);

    let useless = mk(&log, author, "/skills/useless", "---\nname: useless\ndescription: no help\n---\nnothing\n");
    assert!(w(&log).install_skill_evaluated(useless, "cmd", &heldout, &control).is_err(), "held-out fails with skill -> refused");

    let harmful = mk(&log, author, "/skills/harmful", "---\nname: harmful\ndescription: helps then breaks\n---\nmagic-word BREAK\n");
    assert!(w(&log).install_skill_evaluated(harmful, "cmd", &heldout, &control).is_err(), "control regression -> refused");
    assert_eq!(w(&log).skill_index().len(), 1, "only the helpful skill is installed");
}
