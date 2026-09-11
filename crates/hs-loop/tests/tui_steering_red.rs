//! RED contract tests: mid-mission text STEERS the running mission
//! (Eric 2026-09-11: "entering any feedback gets translated as a new
//! mission? queued #1"). Typed feedback lands in the loop's steering
//! inbox - drained at the next step boundary into the model's volatile
//! tail - and the transcript acknowledges it as steering, never as a
//! queued new mission.

#[test]
fn append_steering_appends_lines_to_the_inbox() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = dir.path().join("steering.txt");
    hs_loop::tui::append_steering(&inbox, "use UDP datagrams for lt3").unwrap();
    hs_loop::tui::append_steering(&inbox, "and keep h3 on top").unwrap();
    let body = std::fs::read_to_string(&inbox).unwrap();
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines, ["use UDP datagrams for lt3", "and keep h3 on top"]);
}

#[test]
fn steering_echo_acknowledges_in_the_transcript() {
    let mut st = hs_loop::tui::TuiState::default();
    st.push_steering_echo("use UDP datagrams for lt3");
    let t = st
        .transcript
        .iter()
        .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(t.contains("steering › use UDP datagrams for lt3"), "echo: {t:?}");
    assert!(!t.contains("queued #"), "never a queued mission: {t:?}");
}
