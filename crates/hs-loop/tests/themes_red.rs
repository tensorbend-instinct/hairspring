//! REPL UI gap #9 (Eric 2026-09-08: "just fix the gaps" + 13:12
//! look-and-feel steering): pi/omp ship themes as FILES (dark/light
//! built in) and recolor every surface from them; hs-repl hardcodes its
//! SGR codes at each paint site.
//!
//! Contract: a Theme - named roles mapped to SGR codes - drives every
//! painted surface (cards, status bar, composer, markdown). dark and
//! light ship built in; a TOML file overrides any role and falls back
//! to dark for the rest. `HS_THEME` selects: dark|light|/path/to.toml.

use hs_loop::uipaint::{Painter, Theme, UiEvent};

// R1: dark and light are built in and genuinely different.
#[test]
fn r1_builtin_themes_differ() {
    let dark = Theme::dark();
    let light = Theme::light();
    assert_ne!(dark, light, "light must recolor for light backgrounds");
    for t in [&dark, &light] {
        assert!(!t.accent.is_empty() && !t.ok.is_empty() && !t.fail.is_empty() && !t.dim.is_empty());
    }
}

// R2: a custom theme recolors the tool card: every painted surface
// takes its codes from the theme, not from literals.
#[test]
fn r2_painter_uses_theme_codes() {
    let mut custom = Theme::dark();
    custom.tool = "35".into(); // magenta plugin names
    custom.ok = "34".into();   // blue ok marks
    let mut out: Vec<u8> = Vec::new();
    {
        let mut p = Painter::with_theme(&mut out, true, &custom);
        p.handle(&UiEvent::ToolCallStart {
            plugin: "term.exec".into(),
            args_summary: "echo hi".into(),
        });
        p.handle(&UiEvent::ToolCallEnd {
            plugin: "term.exec".into(),
            ok: true,
            output_summary: "hi".into(),
            elapsed_ms: 1,
        });
    }
    let s = String::from_utf8(out).unwrap();
    assert!(s.contains("\x1b[35m"), "plugin name in the theme's magenta: {s:?}");
    assert!(s.contains("\x1b[34m"), "ok mark in the theme's blue: {s:?}");
}

// R3: TOML files override roles partially; the rest falls back to dark.
#[test]
fn r3_theme_from_toml_partial_override() {
    let t = Theme::from_toml(
        r#"
accent = "35;1"
cost = "36"
"#,
    )
    .expect("valid theme file");
    assert_eq!(t.accent, "35;1");
    assert_eq!(t.cost, "36");
    let dark = Theme::dark();
    assert_eq!(t.ok, dark.ok, "unset roles fall back to dark");
    assert_eq!(t.tool, dark.tool);
}

// R4: a malformed file is an error, not a silent default; unknown roles
// are rejected so typos don't silently no-op.
#[test]
fn r4_theme_from_toml_rejects_garbage() {
    assert!(Theme::from_toml("this is not toml = [").is_err());
    assert!(Theme::from_toml("nope = \"31\"").is_err());
}

// R5: selection by name: dark, light, or a file path.
#[test]
fn r5_theme_by_name() {
    assert_eq!(Theme::by_name("dark").unwrap(), Theme::dark());
    assert_eq!(Theme::by_name("light").unwrap(), Theme::light());
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("mine.toml");
    std::fs::write(&p, "accent = \"33;1\"\n").unwrap();
    let t = Theme::by_name(p.to_str().unwrap()).unwrap();
    assert_eq!(t.accent, "33;1");
    assert!(Theme::by_name("nonexistent-theme-name").is_err());
}
