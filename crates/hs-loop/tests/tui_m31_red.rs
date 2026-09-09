//! Hostile pass 2026-09-08 (post-#5): the :agents panel diverges from
//! its own signed-off design (docs/ui-fullscreen-gate.md, Eric's
//! addendum): nodes must carry the child's MODEL, and the graph must
//! render as an INDENTED TREE (parent->child), not a flat list.
//! Shipped: AgentNode{stream_id, mission, status} - no parent, no
//! model, flat render. This RED pins the documented contract.

use hs_loop::tui::{self, DelegationGraph, TuiState};
use ratatui::{backend::TestBackend, Terminal};

fn rendered(st: &TuiState, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| tui::render_skeleton(f, st)).unwrap();
    let buf = term.backend().buffer();
    let mut out = String::new();
    for y in 0..h {
        for x in 0..w {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

// R1: a node knows its parent and its model - the Spawn provenance,
// not just the child id.
#[test]
fn r1_node_carries_parent_and_model() {
    let mut g = DelegationGraph::new();
    let parent = uuid::Uuid::new_v4();
    let child = uuid::Uuid::new_v4();
    g.note_spawn(child, Some(parent), "research the cache layer", "m-alpha");
    assert_eq!(g.nodes()[0].parent, Some(parent));
    assert_eq!(g.nodes()[0].model, "m-alpha");
}

// R2: the panel renders an indented tree with the model on every
// node - depth from the parent chain, model dim after the mission.
#[test]
fn r2_panel_renders_indented_tree_with_models() {
    let mut st = TuiState::default();
    let c1 = uuid::Uuid::new_v4();
    let c2 = uuid::Uuid::new_v4();
    st.agents.note_spawn(c1, None, "top task", "m-alpha");
    st.agents.note_spawn(c2, Some(c1), "nested task", "m-beta");
    st.toggle_agents_panel();
    let screen = rendered(&st, 100, 30);
    // Both models visible.
    assert!(screen.contains("m-alpha"), "parent model shown:\n{screen}");
    assert!(screen.contains("m-beta"), "child model shown:\n{screen}");
    // Tree: the nested node's line is indented deeper than its parent's.
    let line_of = |id: uuid::Uuid| -> String {
        let short: String = id.to_string().chars().take(8).collect();
        screen
            .lines()
            .find(|l| l.contains(&short))
            .unwrap_or("")
            .to_string()
    };
    let (l1, l2) = (line_of(c1), line_of(c2));
    assert!(!l1.is_empty() && !l2.is_empty(), "both nodes rendered:\n{screen}");
    // Indent = column of the status glyph (the border column is shared).
    let indent_of = |l: &str| {
        l.find(['\u{25b6}', '\u{2713}', '\u{2717}'])
            .expect("status glyph on the node line")
    };
    assert!(
        indent_of(&l2) > indent_of(&l1),
        "nested child indented deeper than parent:\nP:{l1}\nC:{l2}"
    );
}

// R3: scan_stream recovers parent + model from the durable Spawn
// payload - the graph survives resume with its provenance intact.
#[test]
fn r3_scan_stream_recovers_model_and_parent() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let parent = uuid::Uuid::new_v4();
    let child = uuid::Uuid::new_v4();
    let mut pw = hs_log::StreamWriter::create(root, parent).unwrap();
    pw.append(
        hs_core::EventBuilder::new(hs_core::EventKind::Spawn).payload(
            hs_core::Payload::Inline(
                serde_json::to_vec(&serde_json::json!({
                    "child_stream_id": child,
                    "mission": "map the parser",
                    "model": "m-gamma"
                }))
                .unwrap(),
            ),
        ),
    )
    .unwrap();
    drop(pw);
    let g = DelegationGraph::scan_stream(root, parent).unwrap();
    assert_eq!(g.nodes()[0].parent, Some(parent), "parent = scanned stream");
    assert_eq!(g.nodes()[0].model, "m-gamma", "model from the Spawn payload");
}
