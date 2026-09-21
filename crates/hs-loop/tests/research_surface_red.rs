use hs_loop::tui::{TUI_COMMANDS, TUI_HELP, command_lookup, command_matches};
#[test]
fn research_is_reachable_from_help_and_live_palette() {
    assert!(TUI_HELP.contains("/research"));
    let cmd = command_lookup("research").expect("/research must be registered");
    assert!(cmd.summary.contains("research"));
    assert_eq!(
        command_matches("rese")
            .iter()
            .map(|c| c.name)
            .collect::<Vec<_>>(),
        vec!["research"]
    );
    assert!(TUI_COMMANDS.iter().any(|c| c.name == "research"));
}
