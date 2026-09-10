//! B8c RED: the TUI chrome half of checklist 8.2/8.4/8.5 - the read
//! views (B8b) must be REACHABLE from the full-screen surface:
//! `:lineage` / `:scorer` / `:time` commands toggle overlay panels
//! that render the selfmod lineage, the scorer stream, and the
//! session's `T_mission` decomposition. Empty streams render an honest
//! "nothing this session" line, never a blank box.

use hs_loop::mission_time::Decomposition;
use hs_loop::tui::{self, KeyAction, TuiState};
use hs_loop::tui_views::{
    CanaryRecord, CapabilityDelta, FitnessDelta, MutationRecord, ScoreRecord, ScorerPinRecord,
    ScorerView, SelfmodView,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn viewport_text(st: &TuiState, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| tui::render_skeleton(f, st)).unwrap();
    let buf = term.backend().buffer();
    (0..h as usize)
        .map(|y| {
            (0..w)
                .map(|x| buf[(x, y as u16)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn type_and_submit(st: &mut TuiState, cmd: &str) -> KeyAction {
    for c in cmd.chars() {
        tui::handle_key(st, key(c));
    }
    tui::handle_key(st, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
}

fn sample_lineage() -> SelfmodView {
    SelfmodView {
        mutations: vec![MutationRecord {
            fork: uuid::Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap(),
            changes: 2,
        }],
        capability_deltas: vec![CapabilityDelta {
            candidate: "cap-7".to_string(),
            promoted: true,
            prompts: 9,
            tools: 4,
        }],
        fitness_deltas: vec![FitnessDelta {
            candidate: "cap-7".to_string(),
            held_out_pass_rate: 0.875,
        }],
    }
}

fn sample_scorer() -> ScorerView {
    ScorerView {
        pins: vec![ScorerPinRecord {
            version: "v3".to_string(),
            conditions: "hidden-suite".to_string(),
            hash: "abcdef0123456789".to_string(),
        }],
        scores: vec![ScoreRecord {
            tier: "tier01".to_string(),
            candidate: "cand-2".to_string(),
            suite: "hidden".to_string(),
            passed: true,
            correct: 17,
            total: 18,
        }],
        canaries: vec![CanaryRecord {
            id: "cg-1".to_string(),
            ground_truth_good: true,
            scorer_said_good: false,
            error: true,
        }],
    }
}

fn sample_decomposition() -> Decomposition {
    Decomposition {
        n_steps: 4,
        t_model_ms: 2000,
        t_overhead_ms: 400,
        r_failures: 1,
        t_recover_ms: 300,
        c_coord_events: 2,
        c_coord_ms: 50,
        s_stuck_repeats: 1,
        s_stuck_ms: 100,
        wall_ms: 5000,
        unattributed_ms: 2150,
    }
}

// v1: the lineage overlay renders mutations, capability deltas, and
// fitness deltas from the injected view.
#[test]
fn v1_lineage_panel_renders_view() {
    let st = TuiState {
        lineage_view: Some(sample_lineage()),
        lineage_panel: true,
        ..TuiState::default()
    };
    let text = viewport_text(&st, 110, 32);
    assert!(text.contains("lineage"), "panel title: {text:?}");
    assert!(
        text.contains("12345678"),
        "mutation fork short id: {text:?}"
    );
    assert!(text.contains("2 changes"), "mutation changes: {text:?}");
    assert!(text.contains("cap-7"), "candidate name: {text:?}");
    assert!(text.contains("promoted"), "capability delta: {text:?}");
    assert!(
        text.contains("prompts=9") && text.contains("tools=4"),
        "capability counts: {text:?}"
    );
    assert!(text.contains("87.5%"), "held-out pass rate: {text:?}");
}

// v2: the scorer overlay renders pins, scores, and canary results.
#[test]
fn v2_scorer_panel_renders_view() {
    let st = TuiState {
        scorer_view: Some(sample_scorer()),
        scorer_panel: true,
        ..TuiState::default()
    };
    let text = viewport_text(&st, 110, 32);
    assert!(text.contains("scorer"), "panel title: {text:?}");
    assert!(text.contains("v3"), "pin version: {text:?}");
    assert!(text.contains("hidden-suite"), "pin conditions: {text:?}");
    assert!(text.contains("tier01"), "score tier: {text:?}");
    assert!(text.contains("cand-2"), "score candidate: {text:?}");
    assert!(text.contains("17/18"), "score c/t: {text:?}");
    assert!(text.contains("cg-1"), "canary id: {text:?}");
    assert!(
        text.to_lowercase().contains("error"),
        "canary error flagged: {text:?}"
    );
}

// v3: the time overlay renders the session's T_mission decomposition.
#[test]
fn v3_time_panel_renders_decomposition() {
    let st = TuiState {
        time_view: Some(sample_decomposition()),
        time_panel: true,
        ..TuiState::default()
    };
    let text = viewport_text(&st, 110, 32);
    assert!(text.contains("T_mission"), "panel title: {text:?}");
    assert!(text.contains("N_steps=4"), "step count: {text:?}");
    assert!(text.contains("wall_ms=5000"), "wall time: {text:?}");
    assert!(
        text.contains("R_failures=1"),
        "recovery term: {text:?}"
    );
}

// v4: the three commands route through the key handler as surface
// actions with their own KeyAction variants, and toggle both ways.
#[test]
fn v4_commands_toggle_panels() {
    let mut st = TuiState::default();
    assert_eq!(type_and_submit(&mut st, ":lineage"), KeyAction::ToggleLineage);
    assert!(st.lineage_panel, "lineage panel open");
    assert_eq!(type_and_submit(&mut st, ":lineage"), KeyAction::ToggleLineage);
    assert!(!st.lineage_panel, "lineage panel closed");

    assert_eq!(type_and_submit(&mut st, ":scorer"), KeyAction::ToggleScorer);
    assert!(st.scorer_panel, "scorer panel open");

    assert_eq!(type_and_submit(&mut st, ":time"), KeyAction::ToggleTime);
    assert!(st.time_panel, "time panel open");

    assert_eq!(type_and_submit(&mut st, ":evidence"), KeyAction::ToggleEvidence);
    assert!(st.evidence_panel, "evidence panel open");
    assert_eq!(type_and_submit(&mut st, ":evidence"), KeyAction::ToggleEvidence);
    assert!(!st.evidence_panel, "evidence panel closed");
}

// v5: an open panel over an absent stream renders an honest empty
// line - never a blank box or a fabricated stream.
#[test]
fn v5_empty_views_render_honest_lines() {
    let mut st = TuiState::default();
    st.toggle_lineage_panel();
    let text = viewport_text(&st, 110, 32);
    assert!(
        text.contains("no selfmod cycle this session"),
        "lineage empty line: {text:?}"
    );

    let mut st = TuiState::default();
    st.toggle_scorer_panel();
    let text = viewport_text(&st, 110, 32);
    assert!(
        text.contains("no scorer stream this session"),
        "scorer empty line: {text:?}"
    );

    let mut st = TuiState::default();
    st.toggle_time_panel();
    let text = viewport_text(&st, 110, 32);
    assert!(
        text.contains("no mission decomposition yet"),
        "time empty line: {text:?}"
    );

    let mut st = TuiState::default();
    st.toggle_evidence_panel();
    let text = viewport_text(&st, 110, 32);
    assert!(
        text.contains("no evidence claims this session"),
        "evidence empty line: {text:?}"
    );
}

// v6: the surface help names all three commands.
#[test]
fn v6_help_lists_chrome_commands() {
    for cmd in [":lineage", ":scorer", ":time", ":evidence"] {
        assert!(
            tui::TUI_HELP.contains(cmd),
            "TUI_HELP must document {cmd}"
        );
    }
}

// v7: the evidence overlay renders claims (verified, regressed with
// both event refs, superseded) and surfaces unfoldable drift
// regressions under their own heading.
#[test]
fn v7_evidence_overlay_renders_claims_and_unresolved_lines() {
    use hs_loop::tui_views::{
        EvidenceClaimKind, EvidenceClaimStatus, EvidenceClaimView, EvidenceView,
    };
    let v = uuid::Uuid::parse_str("aaaaaaaa-1111-2222-3333-444444444444").unwrap();
    let m = uuid::Uuid::parse_str("bbbbbbbb-1111-2222-3333-444444444444").unwrap();
    let view = EvidenceView {
        claims: vec![
            EvidenceClaimView {
                subject: "cap-7".to_string(),
                kind: EvidenceClaimKind::Verified,
                status: EvidenceClaimStatus::Open,
                verified_at: Some(v),
                regressed_at: None,
            },
            EvidenceClaimView {
                subject: "cap-9".to_string(),
                kind: EvidenceClaimKind::Regression,
                status: EvidenceClaimStatus::Superseded,
                verified_at: Some(v),
                regressed_at: Some(m),
            },
        ],
        unresolved_regressions: vec![
            "regression candidate=cap-9 suite=token-heldout pass_rate=0.500".to_string(),
        ],
    };
    let mut st = TuiState::default();
    st.evidence_view = Some(view);
    st.toggle_evidence_panel();
    let text = viewport_text(&st, 110, 32);
    for needle in ["evidence", "cap-7", "cap-9", "REGRESSED", "superseded", "pass_rate=0.500"] {
        assert!(text.contains(needle), "missing {needle:?}: {text:?}");
    }
}
