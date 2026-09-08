//! UI gap #10 M13 RED: two hostile-review findings from the v2 proof
//! set (cap3 HUD vs done-line, cap5 picker entry).
//!
//! 1. The HUD ended every mission at 2x the real call count: M10
//!    counts calls LIVE via ModelCallEnd, and the bin's Done handler
//!    then added MissionResult.model_calls a second time (cap3:
//!    "done: 2 steps, 2 calls" while the HUD read "4 calls" - same
//!    screen, two truths). Reconciliation moves into
//!    TuiState::mission_done: steps accrue (no live event), calls do
//!    NOT (already counted live), cost takes the session total.
//!
//! 2. Picker previews showed a literal "\n" (cap5 entry read
//!    "fix the parser bug\nand keep the suite green"): the mission
//!    payload is one JSON line, so a multi-line goal reaches
//!    list_sessions as an ESCAPED two-char \n that the real-newline
//!    split never sees. Previews flatten it.

// R1: live call events + mission_done reconcile to the real count.
#[test]
fn r1_mission_done_does_not_double_count_calls() {
    let mut st = hs_loop::tui::TuiState::default();
    for _ in 0..2 {
        st.on_ui_event(&hs_loop::uipaint::UiEvent::ModelCallStart {
            model: "scripted".to_string(),
        });
        st.on_ui_event(&hs_loop::uipaint::UiEvent::ModelCallEnd {
            model: "scripted".to_string(),
            input_tokens: 52,
            output_tokens: 9,
        });
    }
    st.mission_done(2, 1_400);
    assert_eq!(st.missions_run, 1);
    assert_eq!(st.total_steps, 2);
    assert_eq!(
        st.total_model_calls, 2,
        "calls were counted live - mission_done must not add them again"
    );
    assert_eq!(st.total_cost_micros, 1_400, "cost takes the session total");
}

// R2: a multi-line mission goal previews as one clean line.
#[test]
fn r2_picker_preview_flattens_escaped_newlines() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let id = uuid::Uuid::new_v4();
    let mut w = hs_log::StreamWriter::create(root, id).unwrap();
    // GoalUpdate marks the stream as a resumable operator session
    // (M12); the mission text mirrors what repl::run_goal writes into
    // the prompt payload - one JSON line, newline escaped.
    w.append(
        hs_core::EventBuilder::new(hs_core::EventKind::GoalUpdate).payload(
            hs_core::Payload::Inline(
                serde_json::to_vec(&serde_json::json!({
                    "mission": "fix the parser bug\nand keep the suite green",
                    "done": false, "outcome": "steps_exhausted"
                }))
                .unwrap(),
            ),
        ),
    )
    .unwrap();
    w.append(
        hs_core::EventBuilder::new(hs_core::EventKind::ModelCall).payload(
            hs_core::Payload::Inline(
                serde_json::to_vec(&serde_json::json!({
                    "prompt": "MISSION: fix the parser bug\nand keep the suite green\n\n..."
                }))
                .unwrap(),
            ),
        ),
    )
    .unwrap();
    drop(w);

    let infos = hs_loop::repl::list_sessions(root);
    assert_eq!(infos.len(), 1, "operator stream found: {infos:?}");
    assert!(
        !infos[0].preview.contains("\\n"),
        "no literal escape in the preview: {:?}",
        infos[0].preview
    );
    assert!(
        infos[0].preview.contains("fix the parser bug")
            && infos[0].preview.contains("keep the suite green"),
        "both lines survive, flattened: {:?}",
        infos[0].preview
    );
}
