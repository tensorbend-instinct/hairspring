//! Eric 2026-09-10 (via parent): "Is [provider switching] in the
//! palette?" The /models picker switches live but the choice resets on
//! restart - same gap class as /caps. The switch persists: the chosen
//! model's [[models]] entry gains `default = true` and every other
//! entry loses it, so a restarted session comes up on the pick.

/// The writer flips exactly one default across the [[models]] blocks,
/// preserves comments (a commented-out default is NOT an active one),
/// and errors on an unknown model name.
#[test]
fn set_default_model_flips_exactly_one() {
    let base = concat!(
        "# rig comment\n\n",
        "[[models]]\nname = \"deepseek\"\ncommand = [\"/bin/hs-plugin-deepseek\"]\n# default = true\nsubjects = [\"*\"]\n\n",
        "[[models]]\nname = \"scripted\"\ncommand = [\"/bin/hs-plugin-scripted\"]\ndefault = true\nsubjects = [\"*\"]\n",
    );
    let out = hs_loop::repl::set_default_model_text(base, "deepseek").unwrap();
    assert!(out.contains("# rig comment"), "comment preserved: {out}");
    assert!(out.contains("# default = true"), "commented default untouched: {out}");
    // exactly one ACTIVE default line, inside the deepseek block
    let active: Vec<&str> = out
        .lines()
        .filter(|l| l.trim() == "default = true")
        .collect();
    assert_eq!(active.len(), 1, "one active default: {out}");
    // line-exact matching: the commented-out "# default = true" must
    // never count as the active default.
    let line_pos = |text: &str, needle: &str| {
        text.lines().position(|l| l.trim() == needle).unwrap()
    };
    let ds = line_pos(&out, "name = \"deepseek\"");
    let sc = line_pos(&out, "name = \"scripted\"");
    let dflt = line_pos(&out, "default = true");
    assert!(ds < dflt && dflt < sc, "default inside deepseek block: {out}");
    // flipping back is idempotent in shape
    let back = hs_loop::repl::set_default_model_text(&out, "scripted").unwrap();
    let active2: Vec<&str> = back.lines().filter(|l| l.trim() == "default = true").collect();
    assert_eq!(active2.len(), 1, "{back}");
    assert!(
        line_pos(&back, "default = true") > line_pos(&back, "name = \"scripted\""),
        "{back}"
    );
    // unknown model is an error naming the known set
    let e = hs_loop::repl::set_default_model_text(base, "bogus").unwrap_err();
    assert!(e.contains("bogus") && e.contains("deepseek"), "{e}");
}

/// The file-level writer + the restart contract: persist a pick, and a
/// session loaded from that config comes up on the picked model.
#[test]
fn model_pick_persists_and_arms_on_load() {
    let dir = std::env::temp_dir().join("model-persist-load");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("script.jsonl"), "## Done\n").unwrap();
    let cfg = dir.join("hairspring.toml");
    std::fs::write(
        &cfg,
        concat!(
            "[[models]]\nname = \"other\"\ncommand = [\"/bin/nope\"]\n\n",
            "[[models]]\nname = \"scripted\"\ncommand = [\"/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted\"]\ndefault = true\n",
        ),
    )
    .unwrap();
    hs_loop::repl::set_default_model(&cfg, "other").expect("persist other");
    let text = std::fs::read_to_string(&cfg).unwrap();
    assert_eq!(
        text.lines().filter(|l| l.trim() == "default = true").count(),
        1,
        "{text}"
    );
    assert_eq!(
        hs_loop::repl::ReplSession::configured_model_label(&cfg).as_deref(),
        Some("other"),
        "a fresh session reads the persisted default"
    );
    hs_loop::repl::set_default_model(&cfg, "scripted").expect("flip back");
    assert_eq!(
        hs_loop::repl::ReplSession::configured_model_label(&cfg).as_deref(),
        Some("scripted")
    );
}

