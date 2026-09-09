//! Eric's five #4 (2026-09-08): model/theme pickers. The TUI showed
//! the configured model in the composer but offered no way to change
//! it, and the theme was boot-time only (`HS_THEME` env). An operator
//! surface lets you switch both live.
//!
//! Contract (model): `ReplSession::set_model_override` selects which
//! configured model serves operator calls from the next mission
//! onward; an unknown name is rejected with an error naming it, and
//! the override - not config order - decides the call. Contract
//! (theme): `uipaint::available_themes()` catalogs the built-ins and
//! the TUI state can switch between them live.

use hs_loop::repl::load_session;

fn write_fixture(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).unwrap();
    // Two scripted models, identical behavior; the kernel's dispatch
    // record in the stream names the serving plugin, so the ledger
    // itself proves which model served each call.
    let toml = r#"
[[tools]]
name = "answer.submit"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-answersubmit"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-liechecker"]
subjects = ["*"]

[[models]]
name = "m-alpha"
command = ["/bin/sh", "-c", "HS_SCRIPTED_NAME=m-alpha exec /mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted"]
default = true
subjects = ["*"]

[[models]]
name = "m-beta"
command = ["/bin/sh", "-c", "HS_SCRIPTED_NAME=m-beta exec /mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted"]
subjects = ["*"]
"#;
    std::fs::write(dir.join("hairspring.toml"), toml).unwrap();
    std::fs::write(dir.join("script.jsonl"), "reading the code\n").unwrap();
}

/// Every payload in every stream under the run dir, concatenated -
/// the raw ledger, same reader pattern as `blind_mode_red`.
fn ledger_text(log_root: &std::path::Path) -> String {
    let streams = log_root.join("streams");
    let mut all = String::new();
    for e in std::fs::read_dir(&streams).unwrap() {
        let sid = uuid::Uuid::parse_str(&e.unwrap().file_name().to_string_lossy()).unwrap();
        let reader = hs_log::StreamReader::open(log_root, sid).unwrap();
        for ev in reader.events().unwrap() {
            if let Ok(b) = reader.resolve_payload(&ev) {
                all.push_str(&String::from_utf8_lossy(&b));
                all.push('\n');
            }
        }
    }
    all
}

// R1: the override decides which model serves the mission. Both
// missions pass; the ledger's model.call dispatch records prove the
// serving model: default run names m-alpha, override run names
// m-beta, and never the other.
#[test]
fn r1_model_override_decides_the_call() {
    std::env::set_var("HS_ANSWER_RAW", "1");
    std::env::set_var("HS_SCRIPTED_PROMPT_AWARE", "1");
    let dir = std::env::temp_dir().join("model-override-r1");
    let _ = std::fs::remove_dir_all(&dir);
    write_fixture(&dir);
    std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl"));

    let mut s = load_session(&dir.join("hairspring.toml"), &dir.join("run-a"), false, 5, None, None)
        .unwrap();
    let d = s.run_goal("fix the lexer").unwrap();
    assert!(d.passed, "default run passes: {d:?}");
    let led_a = ledger_text(&dir.join("run-a"));
    assert!(led_a.contains("\"plugin\": \"m-alpha\"") || led_a.contains("\"plugin\":\"m-alpha\""),
        "default run served by m-alpha");
    assert!(!led_a.contains("m-beta"), "default run never touches m-beta");

    let mut s = load_session(&dir.join("hairspring.toml"), &dir.join("run-b"), false, 5, None, None)
        .unwrap();
    s.set_model_override(Some("m-beta".to_string())).unwrap();
    let good = s.run_goal("fix the lexer").unwrap();
    assert!(good.passed, "override run passes: {good:?}");
    let led_b = ledger_text(&dir.join("run-b"));
    assert!(led_b.contains("\"plugin\": \"m-beta\"") || led_b.contains("\"plugin\":\"m-beta\""),
        "override run served by m-beta");
    assert!(!led_b.contains("m-alpha"), "override run never touches m-alpha");
}

// R2: an unknown model name is rejected with an error naming it;
// the configured names are listable for the picker.
#[test]
fn r2_unknown_model_rejected_and_names_listed() {
    let dir = std::env::temp_dir().join("model-override-r2");
    let _ = std::fs::remove_dir_all(&dir);
    write_fixture(&dir);
    let mut s = load_session(&dir.join("hairspring.toml"), &dir.join("run"), false, 5, None, None)
        .unwrap();
    let err = s
        .set_model_override(Some("nope".to_string()))
        .expect_err("unknown model must be rejected");
    assert!(err.to_string().contains("nope"), "error names it: {err}");
    let names = s.model_names();
    assert_eq!(
        names,
        vec![("m-alpha".to_string(), true), ("m-beta".to_string(), false)],
        "picker entries: (name, is_default), stable sorted order"
    );
    // Clearing the override returns to the config default.
    s.set_model_override(None).unwrap();
}

// R3: the theme catalog lists the built-ins and the TUI state
// switches between them live.
#[test]
fn r3_theme_catalog_and_live_switch() {
    let themes = hs_loop::uipaint::available_themes();
    let names: Vec<&str> = themes.iter().map(|(n, _)| *n).collect();
    assert_eq!(names, ["dark", "light"]);
    assert_ne!(themes[0].1.accent, themes[1].1.accent, "themes differ");
    let mut st = hs_loop::tui::TuiState::default();
    assert_eq!(st.theme, hs_loop::uipaint::Theme::dark());
    st.set_theme("light", themes[1].1.clone());
    assert_eq!(st.theme, themes[1].1);
    let t: String = st
        .transcript
        .iter()
        .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
        .collect();
    assert!(t.contains("light"), "switch echoes the theme name: {t:?}");
}

// R4 (found by the live proof): the picker overlay titled itself
// "resume" for every kind. The title names what is being chosen.
#[test]
fn r4_picker_title_names_the_kind() {
    use hs_loop::tui::{self, PickerKind, TuiState};
    use ratatui::{backend::TestBackend, Terminal};
    let render = |st: &TuiState| -> String {
        let backend = TestBackend::new(60, 16);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| tui::render_skeleton(f, st)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..16)
            .map(|y| {
                (0..60)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut st = TuiState::default();
    st.open_picker_kind(PickerKind::Models, vec!["m-alpha (current)".into(), "m-beta".into()]);
    let screen = render(&st);
    assert!(screen.contains("╭ models"), "models picker titled: {screen:?}");
    assert!(!screen.contains("╭ resume"), "not the resume title: {screen:?}");
    st.open_picker_kind(PickerKind::Themes, vec!["dark".into(), "light".into()]);
    assert!(render(&st).contains("╭ theme"), "theme picker titled");
}
