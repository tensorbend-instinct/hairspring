//! Eric 2026-09-10 (iMessage, via parent): teach the tar userns quirk to
//! agents - "harness hint plus docs, whatever reaches the mission at the
//! right moment." Also fixes the stale SWE template claim "System roots
//! are writable" (false since the 88dc0a4 ro-floor ruling fix).

/// The REPL mission's first message carries the environment hint so every
/// mission learns the rules before its first step.
#[test]
fn repl_mission_message_carries_env_hint() {
    let m = hs_loop::msgfmt::mission_first_message("fix the parser");
    assert!(m.contains("MISSION: fix the parser"), "{m}");
    for needle in [
        "read-only",
        "tar --no-same-owner",
        "workspace",
        "writable",
    ] {
        assert!(m.contains(needle), "missing {needle}: {m}");
    }
}

/// The hint explains WHY: userns-root tar chowns to unmapped uids and
/// floods one warning per file otherwise.
#[test]
fn env_hint_explains_tar_quirk() {
    let h = hs_loop::msgfmt::MISSION_ENV_HINT;
    assert!(h.contains("tar --no-same-owner"), "{h}");
    assert!(h.contains("chown") || h.contains("ownership"), "{h}");
}

/// The SWE templates stopped claiming writable system roots (stale since
/// the ro-floor fix) and teach the workspace-install + tar hint instead.
#[test]
fn swe_templates_tell_the_truth() {
    for t in [
        hs_loop::sweprompt::SWE_MISSION_TEMPLATE,
        hs_loop::sweprompt::SWE_MISSION_BLIND_TEMPLATE,
        hs_loop::sweprompt::TB_MISSION_TEMPLATE,
    ] {
        assert!(!t.contains("System roots are writable"), "stale claim: {t}");
        assert!(t.contains("tar --no-same-owner"), "missing tar hint: {t}");
        assert!(t.contains("read-only"), "missing ro statement: {t}");
    }
}
