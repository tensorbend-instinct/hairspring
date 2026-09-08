//! UI gap #10 M6: the delegation graph. hs-swarm records delegations as
//! Spawn events on the parent stream (child_stream_id + mission) and
//! the child's completion as GoalUpdate {done:true} on the child
//! stream. The full-screen surface renders that as a live agents panel:
//! first-class loop substrate, not a bolt-on.

use hs_loop::tui::{self, DelegationGraph, TuiState};
use ratatui::{backend::TestBackend, Terminal};

// R1: nodes appear on spawn, newest knowledge of status wins.
#[test]
fn r1_spawn_and_completion() {
    let mut g = DelegationGraph::new();
    let child = uuid::Uuid::new_v4();
    g.note_spawn(child, "research the cache layer");
    assert_eq!(g.nodes().len(), 1);
    assert_eq!(g.nodes()[0].mission, "research the cache layer");
    assert_eq!(g.nodes()[0].status, tui::AgentStatus::Running);
    g.note_done(child, true);
    assert_eq!(g.nodes()[0].status, tui::AgentStatus::Done);
    g.note_done(child, false);
    assert_eq!(g.nodes()[0].status, tui::AgentStatus::Failed);
}

// R2: scan_stream derives the graph from the durable log - Spawn on
// the parent, completion from the child's own GoalUpdate.
#[test]
fn r2_scan_stream_from_substrate() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let parent = uuid::Uuid::new_v4();
    let child = uuid::Uuid::new_v4();
    // Child stream with a completed goal (mirrors hs_swarm::Spawner).
    let mut cw = hs_log::StreamWriter::create(root, child).unwrap();
    cw.append(
        hs_core::EventBuilder::new(hs_core::EventKind::GoalUpdate).payload(
            hs_core::Payload::Inline(
                serde_json::to_vec(&serde_json::json!({
                    "mission": "map the parser", "child_of": parent, "done": true
                }))
                .unwrap(),
            ),
        ),
    )
    .unwrap();
    drop(cw);
    // Parent records the delegation.
    let mut pw = hs_log::StreamWriter::create(root, parent).unwrap();
    pw.append(
        hs_core::EventBuilder::new(hs_core::EventKind::Spawn).payload(
            hs_core::Payload::Inline(
                serde_json::to_vec(&serde_json::json!({
                    "child_stream_id": child, "mission": "map the parser"
                }))
                .unwrap(),
            ),
        ),
    )
    .unwrap();
    drop(pw);

    let g = DelegationGraph::scan_stream(root, parent).unwrap();
    assert_eq!(g.nodes().len(), 1);
    assert_eq!(g.nodes()[0].mission, "map the parser");
    assert_eq!(g.nodes()[0].stream_id, child);
    assert_eq!(g.nodes()[0].status, tui::AgentStatus::Done, "child GoalUpdate done:true");
}

// R3: the agents panel renders as an overlay with per-node status
/// glyphs, and only exists when the graph is non-empty.
#[test]
fn r3_agents_panel_overlay() {
    let backend = TestBackend::new(80, 24);
    let mut term = Terminal::new(backend).unwrap();
    let mut st = TuiState::default();
    // No delegation: no panel even when toggled.
    st.toggle_agents_panel();
    term.draw(|f| tui::render_skeleton(f, &st)).unwrap();
    {
        let buf = term.backend().buffer();
        let mut all = String::new();
        for y in 0..24u16 {
            for x in 0..80u16 {
                all.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(!all.contains("agents"), "empty graph renders no panel");
    }

    let child = uuid::Uuid::new_v4();
    st.agents.note_spawn(child, "delegate the lexer rewrite");
    st.toggle_agents_panel();
    st.toggle_agents_panel(); // was toggled on above with empty graph; ensure open
    term.draw(|f| tui::render_skeleton(f, &st)).unwrap();
    let short: String = child.to_string().chars().take(8).collect();
    let mut found = (false, false);
    for y in 0..24 {
        let row: String = (0..80).map(|x| term.backend().buffer()[(x, y)].symbol()).collect();
        if row.contains(&short) {
            found.0 = true;
        }
        if row.contains("delegate the lexer rewrite") {
            found.1 = true;
        }
    }
    assert!(found.0 && found.1, "panel shows short id + mission");
}

// R4: a mission UI event carries spawns to the surface - the UiEvent
// seam extended with SubAgentSpawned / SubAgentFinished.
#[test]
fn r4_ui_event_spawn_seam() {
    let mut st = TuiState::default();
    let child = uuid::Uuid::new_v4();
    st.on_ui_event(&hs_loop::uipaint::UiEvent::SubAgentSpawned {
        child,
        mission: "scout the failing test".into(),
    });
    assert_eq!(st.agents.nodes().len(), 1);
    st.on_ui_event(&hs_loop::uipaint::UiEvent::SubAgentFinished { child, ok: true });
    assert_eq!(st.agents.nodes()[0].status, tui::AgentStatus::Done);
}
