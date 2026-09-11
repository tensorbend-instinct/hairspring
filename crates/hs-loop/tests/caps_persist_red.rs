//! Eric 2026-09-10 (via parent, after the /caps round): "/caps changes
//! should persist across sessions. A setting that silently resets is a
//! gap." /caps writes the config file; a restarted session (including
//! the real :resume path) arms what the file holds.

/// The TOML upsert: section created when missing, key replaced in
/// place when present, everything else (comments, other keys, other
/// sections) preserved byte-for-byte.
#[test]
fn toml_upsert_creates_replaces_and_preserves() {
    let base = "# rig comment\n\n[[tools]]\nname = \"answer.write\"\n";
    // create section + key
    let out = hs_loop::repl::toml_upsert(base, "run", "max_steps", Some("70")).unwrap();
    assert!(out.contains("# rig comment"), "comment preserved: {out}");
    assert!(out.contains("[[tools]]"), "other section preserved: {out}");
    assert!(out.contains("[run]\nmax_steps = 70"), "section+key added: {out}");
    // replace in place: exactly one max_steps line, new value
    let out2 = hs_loop::repl::toml_upsert(&out, "run", "max_steps", Some("90")).unwrap();
    assert_eq!(out2.matches("max_steps").count(), 1, "no duplicate: {out2}");
    assert!(out2.contains("max_steps = 90"), "replaced: {out2}");
    assert!(out2.contains("# rig comment"), "still preserved: {out2}");
    // remove (wall off)
    let with_wall = hs_loop::repl::toml_upsert(&out2, "run", "wall_secs", Some("900")).unwrap();
    assert!(with_wall.contains("wall_secs = 900"), "{with_wall}");
    let off = hs_loop::repl::toml_upsert(&with_wall, "run", "wall_secs", None).unwrap();
    assert!(!off.contains("wall_secs"), "key removed: {off}");
    assert!(off.contains("max_steps = 90"), "sibling kept: {off}");
    // a second section upsert does not touch [run]
    let crit = hs_loop::repl::toml_upsert(&off, "critic", "max_steps", Some("24")).unwrap();
    assert!(crit.contains("[critic]\nmax_steps = 24"), "{crit}");
    assert!(crit.contains("[run]\nmax_steps = 90"), "{crit}");
}

/// The config reads: [run] max_steps/wall_secs and the [critic] caps.
#[test]
fn config_cap_readers() {
    let dir = std::env::temp_dir().join("caps-persist-readers");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("hairspring.toml");
    std::fs::write(
        &cfg,
        "[run]\nmax_steps = 70\nwall_secs = 900\nbudget_usd = 2.5\n\n[critic]\nmax_steps = 24\nwall_secs = 300\nbudget_micros = 250000\n",
    )
    .unwrap();
    assert_eq!(hs_loop::repl::configured_max_steps(&cfg), Some(70));
    assert_eq!(hs_loop::repl::configured_wall_secs(&cfg), Some(900));
    let (s, w, b) = hs_loop::repl::configured_critic_caps(&cfg);
    assert_eq!((s, w, b), (Some(24), Some(300), Some(250_000)));
    let empty = dir.join("empty.toml");
    std::fs::write(&empty, "# nothing\n").unwrap();
    assert_eq!(hs_loop::repl::configured_max_steps(&empty), None);
    assert_eq!(hs_loop::repl::configured_wall_secs(&empty), None);
    assert_eq!(
        hs_loop::repl::configured_critic_caps(&empty),
        (None, None, None)
    );
}

fn persisted_session(
    dir: &std::path::Path,
    max_steps_arg: u32,
) -> hs_loop::repl::ReplSession {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("script.jsonl"),
        "## Done\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("hairspring.toml"),
        concat!(
            "[run]\nmax_steps = 70\nwall_secs = 900\nbudget_usd = 2.5\n\n",
            "[critic]\nmax_steps = 24\nwall_secs = 300\nbudget_micros = 250000\n\n",
            "[[models]]\nname = \"scripted\"\n",
            "command = [\"/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted\"]\ndefault = true\n",
        ),
    )
    .unwrap();
    unsafe {
        std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl"));
    }
    hs_loop::repl::load_session(
        &dir.join("hairspring.toml"),
        &dir.join("run"),
        false,
        max_steps_arg,
        None,
        None,
    )
    .unwrap()
}

/// [critic] caps arm as PROCESS-GLOBAL env: the tests that load a
/// config carrying them serialize on this lock.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A session loaded from a config with persisted caps ARMS them
/// (restart-survival at the library level): operator caps on the loop,
/// critic caps as the env the critic reads at spawn.
#[test]
fn persisted_caps_arm_on_load() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join("caps-persist-load");
    let s = persisted_session(&dir, hs_loop::DEFAULT_MISSION_MAX_STEPS);
    let snap = s.caps_snapshot();
    assert_eq!(snap.steps, 70, "[run] max_steps armed: {snap:?}");
    assert_eq!(snap.wall_secs, Some(900), "[run] wall_secs armed: {snap:?}");
    assert_eq!(snap.budget_micros, Some(2_500_000), "[run] budget: {snap:?}");
    assert_eq!(snap.critic_steps, 24, "[critic] max_steps as env: {snap:?}");
    assert_eq!(snap.critic_wall_secs, 300, "{snap:?}");
    assert_eq!(snap.critic_budget_micros, 250_000, "{snap:?}");
    // restore process-global critic env for other tests
    unsafe {
        std::env::remove_var("HS_CRITIC_MAX_STEPS");
        std::env::remove_var("HS_CRITIC_WALL_SECS");
        std::env::remove_var("HS_CRITIC_BUDGET_MICROS");
    }
}

/// An explicit --max-steps still beats the config; only the DEFAULT
/// yields to the persisted value.
#[test]
fn explicit_cli_max_steps_beats_config() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join("caps-persist-cli-wins");
    let s = persisted_session(&dir, 5);
    assert_eq!(s.caps_snapshot().steps, 5, "explicit flag wins");
    unsafe {
        std::env::remove_var("HS_CRITIC_MAX_STEPS");
        std::env::remove_var("HS_CRITIC_WALL_SECS");
        std::env::remove_var("HS_CRITIC_BUDGET_MICROS");
    }
}

/// /caps through the session writes the config file; a session loaded
/// later from that file arms the changed cap (the restart contract).
#[test]
fn set_cap_persists_and_survives_reload() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join("caps-persist-setcap");
    let mut s = persisted_session(&dir, hs_loop::DEFAULT_MISSION_MAX_STEPS);
    s.set_cap("steps", "77").expect("set steps");
    s.set_cap("budget", "3.25").expect("set budget");
    s.set_cap("critic-steps", "30").expect("set critic-steps");
    s.set_cap("wall", "off").expect("wall off");
    drop(s);
    let text = std::fs::read_to_string(dir.join("hairspring.toml")).unwrap();
    assert!(text.contains("max_steps = 77"), "steps persisted: {text}");
    assert!(text.contains("budget_usd = 3.25"), "budget persisted: {text}");
    assert!(!text.contains("wall_secs = 900"), "wall removed: {text}");
    assert!(text.contains("[critic]"), "critic section: {text}");
    let s2 = hs_loop::repl::load_session(
        &dir.join("hairspring.toml"),
        &dir.join("run2"),
        false,
        hs_loop::DEFAULT_MISSION_MAX_STEPS,
        None,
        None,
    )
    .unwrap();
    let snap = s2.caps_snapshot();
    assert_eq!(snap.steps, 77, "steps survived reload: {snap:?}");
    assert_eq!(snap.budget_micros, Some(3_250_000), "budget survived: {snap:?}");
    assert_eq!(snap.wall_secs, None, "wall stayed off: {snap:?}");
    assert_eq!(snap.critic_steps, 30, "critic steps survived: {snap:?}");
    unsafe {
        std::env::remove_var("HS_CRITIC_MAX_STEPS");
        std::env::remove_var("HS_CRITIC_WALL_SECS");
        std::env::remove_var("HS_CRITIC_BUDGET_MICROS");
    }
}
