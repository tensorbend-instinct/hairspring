//! D12 RED (run-of-record 2026-09-09 HUD audit, stream afe9fbf3): the
//! live HUD booked HARDCODED token rates ($3/$15 per 1M tokens,
//! `COST_MICROS_PER_*`) - on the cached `DeepSeek` run it read $9.5077 live
//! while the stream's provider-reported cost summed to $0.7358 (13x
//! drift), and the done/resume path books the recorded `cost_usd_micros`,
//! so live and done told two truths - the exact inconsistency tui.rs's
//! own comment bans ("the done line must show the SAME cost the live HUD
//! showed"). `ModelCallEnd` now carries the provider-reported
//! `cost_usd_micros` and the HUD books it; the token-rate constants are
//! gone.

/// The live HUD shows what the provider reported, not a rate guess.
#[test]
fn live_hud_books_provider_reported_cost_not_a_token_rate_estimate() {
    let mut st = hs_loop::tui::TuiState::default();
    st.on_ui_event(&hs_loop::uipaint::UiEvent::ModelCallStart {
        model: "deepseek".to_string(),
    });
    st.on_ui_event(&hs_loop::uipaint::UiEvent::ModelCallEnd {
        model: "deepseek".to_string(),
        input_tokens: 172_830,
        output_tokens: 512,
        cost_usd_micros: 10_434,
    });
    // old rates would read 172830*3 + 512*15 = $0.5260; the provider said $0.0104.
    assert!(
        st.hud_line().contains("$0.0104"),
        "the HUD books the provider-reported cost: {}",
        st.hud_line()
    );
}
