//! RED (deep-pass hostile review, hs-loop + hs-swarm): the mission id names
//! the mission's work dir (`log_root/work/<id>/answer.txt`). On the swarm
//! child path (hs-plugin-swarm -> `Spawner::spawn_child` -> `run_mission`) the
//! id IS the model's untrusted `agent.spawn` mission string - no slug
//! filter runs on it (the REPL path slugifies; the child path does not).
//! An id carrying a separator or `..` walks the work dir OUT of the
//! substrate and `create_dir_all` happily lands it there: a model tool arg
//! becomes an arbitrary-directory-create + answer-write outside the log
//! root. Mission ids are single safe path components, enforced at the
//! write site.
//!
//! m1: a `../..` id is refused, and nothing is created outside the root.
//! m2: an absolute-path id is refused.
//! m3: a normal id still runs (the guard kills no legitimate mission).

const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");

fn rig(dir: &std::path::Path, log: &std::path::Path, max_steps: u32) -> hs_loop::InnerLoop {
    std::fs::create_dir_all(dir).unwrap();
    let config = dir.join("hairspring.toml");
    std::fs::write(
        &config,
        format!(
            "[[models]]\nname = \"scripted\"\ncommand = [\"{SCRIPTED}\"]\ndefault = true\nsubjects = [\"*\"]\n"
        ),
    )
    .unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    hs_loop::InnerLoop::new(kernel, log, true, max_steps).unwrap()
}

#[test]
fn m1_traversal_mission_id_refused_and_writes_nothing_outside() {
    let dir = std::env::temp_dir().join("mission-path-m1");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Hermetic preflight: the scripted plugin's preflight reads the
    // PROCESS-GLOBAL HS_SEQMODEL_SCRIPT, which only m3 sets - m1/m2
    // used to pass only when thread scheduling ran m3's set_var first
    // (observed failing in a full workspace run, 2026-09-10). The
    // traversal id is refused before any model call, so the script is
    // never read; it only has to exist for preflight.
    let script = dir.join("unused.jsonl");
    std::fs::write(&script, "\"never read\"\n").unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let log = dir.join("run");
    let escape = dir.join("escape-hatch");
    let mut l = rig(&dir.join("cfg"), &log, 4);
    let r = l.run_mission_full("../../escape-hatch", "probe the guard");
    let err = r.err().unwrap_or_else(|| panic!("a traversal id must be refused"));
    let msg = err.to_string();
    assert!(
        !escape.exists(),
        "the traversal created a directory OUTSIDE the substrate: {escape:?}"
    );
    assert!(
        msg.contains("path component"),
        "the refusal says why: {msg}"
    );
}

#[test]
fn m2_absolute_mission_id_refused() {
    // also clear the crime scene a PRE-GUARD run may have left
    let _ = std::fs::remove_dir_all("/tmp/abs-mission-probe");
    let dir = std::env::temp_dir().join("mission-path-m2");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Same hermetic-preflight fix as m1 (process-global env, not thread
    // scheduling).
    let script = dir.join("unused.jsonl");
    std::fs::write(&script, "\"never read\"\n").unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let mut l = rig(&dir.join("cfg"), &dir.join("run"), 4);
    let r = l.run_mission_full("/tmp/abs-mission-probe", "probe");
    let err = r.err().unwrap_or_else(|| panic!("an absolute id must be refused"));
    assert!(
        !std::path::Path::new("/tmp/abs-mission-probe").exists(),
        "the absolute id created a directory outside the substrate"
    );
    assert!(err.to_string().contains("path component"), "{err}");
}

#[test]
fn m3_normal_mission_id_still_runs() {
    let dir = std::env::temp_dir().join("mission-path-m3");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // one prose completion: mission burns its single step
    let script = dir.join("script.jsonl");
    std::fs::write(&script, "\"no tool call here\"\n").unwrap();
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let mut l = rig(&dir.join("cfg"), &dir.join("run"), 1);
    let r = l.run_mission_full("task-1", "ordinary mission").unwrap();
    assert!(!r.passed, "1-step prose completion exhausts: {r:?}");
    assert!(
        dir.join("run").join("work").join("task-1").join("answer.txt")
            .parent()
            .unwrap_or(std::path::Path::new(""))
            .is_dir(),
        "the work dir lands INSIDE the substrate"
    );
}
