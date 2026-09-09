//! Eric's five #1 (2026-09-08): input submitted while a mission runs
//! was DROPPED with "(mission in flight - queued input is a later
//! milestone)". A real operator surface queues: the goal the user
//! typed is work they asked for; swallowing it makes the TUI
//! unusable mid-mission.
//!
//! Contract: goals submitted while a mission is in flight queue in
//! FIFO order; each queue push echoes its position so the operator
//! sees the work was accepted; when the running mission finishes the
//! head of the queue runs next. Slash-commands stay immediate
//! (read-only UI ops); only plain goals queue. The queue lives on
//! TuiState so the bin's event loop stays a thin wiring layer.

use hs_loop::tui::TuiState;

fn transcript_text(st: &TuiState) -> String {
    st.transcript
        .iter()
        .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
        .collect::<Vec<_>>()
        .join("\n")
}

// R1: two goals queued mid-mission drain FIFO, each push echoed with
// its queue position.
#[test]
fn r1_goals_queue_fifo_with_position_echo() {
    let mut st = TuiState::default();
    let p1 = st.queue_goal("fix the lexer");
    let p2 = st.queue_goal("document the module");
    assert_eq!((p1, p2), (1, 2), "queue positions are 1-based");
    let t = transcript_text(&st);
    assert!(t.contains("queued #1"), "first push echoed: {t:?}");
    assert!(t.contains("queued #2"), "second push echoed: {t:?}");
    assert!(t.contains("fix the lexer"), "echo names the goal: {t:?}");
    assert_eq!(st.queued_count(), 2);
    assert_eq!(st.next_queued_goal().as_deref(), Some("fix the lexer"));
    assert_eq!(st.next_queued_goal().as_deref(), Some("document the module"));
    assert_eq!(st.next_queued_goal(), None, "drained queue stays drained");
    assert_eq!(st.queued_count(), 0);
}

// R2: after a full drain the next queue push is position #1 again -
// positions describe the CURRENT queue, not a session lifetime.
#[test]
fn r2_positions_reset_after_drain() {
    let mut st = TuiState::default();
    st.queue_goal("one");
    assert_eq!(st.next_queued_goal().as_deref(), Some("one"));
    assert_eq!(st.queue_goal("two"), 1, "fresh queue starts at #1");
    assert_eq!(st.queued_count(), 1);
}
