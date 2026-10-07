//! RED (2026-10-06 usability comparison): `context_remaining_pct` was
//! never set in production, so the "N% context" readout never appeared.
//! It is now derived from the LAST call's prompt size against the model's
//! known window; an unknown model shows nothing (never a guess).

fn end(st: &mut hs_loop::tui::TuiState, model: &str, input: u64) {
    st.on_ui_event(&hs_loop::uipaint::UiEvent::ModelCallEnd {
        model: model.to_string(),
        input_tokens: input,
        output_tokens: 100,
        cost_usd_micros: 1,
    });
}

#[test]
fn known_model_sets_context_remaining_from_last_prompt() {
    let mut st = hs_loop::tui::TuiState::default();
    end(&mut st, "deepseek-v4-pro", 250_000);
    assert_eq!(st.context_remaining_pct, Some(75));
    end(&mut st, "deepseek-v4-pro", 30_000);
    assert_eq!(st.context_remaining_pct, Some(97));
}

#[test]
fn unknown_model_shows_no_context_percent() {
    let mut st = hs_loop::tui::TuiState::default();
    end(&mut st, "some-unlisted-model", 250_000);
    assert_eq!(st.context_remaining_pct, None);
}

#[test]
fn overfull_prompt_clamps_to_zero() {
    let mut st = hs_loop::tui::TuiState::default();
    end(&mut st, "deepseek-v4-pro", 2_000_000);
    assert_eq!(st.context_remaining_pct, Some(0));
}
