//! Eric 2026-09-12: "The caps should start in restricted (no caps) but
//! during set up or writhing the TUI it should be settable, I'm saying
//! this because the caps doesn't work.. I set 200 steps and 100 dollars
//! on a task and it stoped at a cap of 10 dollars which was strange."
//!
//! Two rulings land here:
//! 1. BUG (his $100 vs the hidden $10): a whole-dollar /caps budget
//!    persisted as a TOML *integer* (`budget_usd = 100`); the config
//!    reader only accepted floats, so the next session silently ignored
//!    his setting and re-armed the hidden $10 default. Explicit
//!    settings must win: the reader takes integer or float dollars and
//!    the writer always emits a decimal literal.
//! 2. CHANGE: a fresh rig arms NO caps - no step cap, no wall, no
//!    budget. Caps are opt-in via `hairspring setup` or /caps, and
//!    "off" disarms one again.

/// [critic] caps arm as PROCESS-GLOBAL env: session-loading tests
/// serialize on this lock.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn scripted_session(
    dir: &std::path::Path,
    rig: &str,
    max_steps_arg: Option<u32>,
) -> hs_loop::repl::ReplSession {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("script.jsonl"), "## Done\n").unwrap();
    std::fs::write(dir.join("hairspring.toml"), rig).unwrap();
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

const SCRIPTED_MODEL: &str = concat!(
    "[[models]]\nname = \"scripted\"\n",
    "command = [\"/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted\"]\ndefault = true\n",
);

/// The bug Eric hit: /caps budget 100 on one session, $10 kill on the
/// next. Whole-dollar budgets must persist as a decimal literal AND a
/// pre-existing integer literal must still read back - both directions,
/// because his config already holds the integer form.
#[test]
fn whole_dollar_budget_survives_restart() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join("caps-default-whole-dollar");
    let mut s = scripted_session(&dir, SCRIPTED_MODEL, None);
    s.set_cap("budget", "100").expect("set budget 100");
    let text = std::fs::read_to_string(dir.join("hairspring.toml")).unwrap();
    assert!(
        text.contains("budget_usd = 100.0"),
        "whole-dollar budget persists as a float literal, not an integer: {text}"
    );
    drop(s);
    let dir2 = std::env::temp_dir().join("caps-default-whole-dollar-int");
    // A config that already holds the integer form (every rig /caps
    // wrote before this fix) reads back too.
    let s2 = scripted_session(
        &dir2,
        "[run]\nbudget_usd = 100\n\n[[models]]\nname = \"scripted\"\ncommand = [\"/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted\"]\ndefault = true\n",
        None,
    );
    assert_eq!(
        s2.caps_snapshot().budget_micros,
        Some(100_000_000),
        "integer budget_usd = 100 arms $100, not the $10 default"
    );
    let s3 = hs_loop::repl::load_session(
        &dir.join("hairspring.toml"),
        &dir.join("run3"),
        false,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        s3.caps_snapshot().budget_micros,
        Some(100_000_000),
        "his $100 survives the restart verbatim"
    );

}

/// A fresh rig (no [run] section, the zero-config stranger path) arms
/// NO caps: no step cap, no wall, no budget. The hidden $10 session
/// default is gone - uncapped is the declared default, caps are opt-in.
#[test]
fn fresh_rig_arms_no_caps() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join("caps-default-fresh");
    let s = scripted_session(&dir, SCRIPTED_MODEL, None);
    let snap = s.caps_snapshot();
    assert_eq!(snap.steps, None, "no default step cap: {snap:?}");
    assert_eq!(snap.wall_secs, None, "no default wall: {snap:?}");
    assert_eq!(snap.budget_micros, None, "no default budget cap: {snap:?}");
}

/// /caps <key> off disarms a cap live AND removes it from the config,
/// so a restart stays uncapped. (wall already had off; steps and
/// budget gain it.)
#[test]
fn caps_off_disarms_and_persists() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join("caps-default-off");
    let mut s = scripted_session(&dir, SCRIPTED_MODEL, None);
    s.set_cap("steps", "200").expect("steps 200");
    s.set_cap("budget", "100").expect("budget 100");
    assert_eq!(s.caps_snapshot().steps, Some(200));
    assert_eq!(s.caps_snapshot().budget_micros, Some(100_000_000));
    s.set_cap("steps", "off").expect("steps off");
    s.set_cap("budget", "off").expect("budget off");
    let snap = s.caps_snapshot();
    assert_eq!(snap.steps, None, "steps disarmed live: {snap:?}");
    assert_eq!(snap.budget_micros, None, "budget disarmed live: {snap:?}");
    let text = std::fs::read_to_string(dir.join("hairspring.toml")).unwrap();
    assert!(!text.contains("max_steps"), "steps line removed: {text}");
    assert!(!text.contains("budget_usd"), "budget line removed: {text}");
    drop(s);
    let s2 = hs_loop::repl::load_session(
        &dir.join("hairspring.toml"),
        &dir.join("run2"),
        false,
        None,
        None,
        None,
    )
    .unwrap();
    let snap2 = s2.caps_snapshot();
    assert_eq!(snap2.steps, None, "steps stay off after restart: {snap2:?}");
    assert_eq!(snap2.budget_micros, None, "budget stays off: {snap2:?}");
}

/// The setup wizard's caps step: answers land in the rig's [run]
/// section with the same literal discipline as /caps; blank answers
/// (the default) leave the config untouched - no caps written.
#[test]
fn setup_caps_answers_apply_to_config() {
    let base = "# rig\n\n[[models]]\nname = \"scripted\"\n";
    let (out, notes) = hs_loop::setup::apply_caps_answers(base, "200", "100").unwrap();
    assert!(out.contains("[run]"), "run section created: {out}");
    assert!(out.contains("max_steps = 200"), "step cap written: {out}");
    assert!(
        out.contains("budget_usd = 100.0"),
        "whole-dollar budget as float literal: {out}"
    );
    assert_eq!(notes.len(), 2, "both caps reported: {notes:?}");
    let (untouched, notes2) = hs_loop::setup::apply_caps_answers(base, "", "").unwrap();
    assert_eq!(untouched, base, "blank answers write nothing");
    assert!(notes2.is_empty(), "nothing to report: {notes2:?}");
    assert!(
        hs_loop::setup::apply_caps_answers(base, "soon", "").is_err(),
        "a non-number step answer is rejected, not written"
    );
    assert!(
        hs_loop::setup::apply_caps_answers(base, "", "lots").is_err(),
        "a non-number budget answer is rejected, not written"
    );
}
