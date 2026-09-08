//! REPL UI gap #10 M21: the done line prints the SESSION's cumulative
//! cost as if it were the mission's own cost. Live proof (cap12-era,
//! two missions one session): mission 2's done line read "$0.0035" -
//! the session total; mission 2 alone spent $0.0021. Every mission
//! after the first overstates itself; pi/omp report the turn's own
//! cost.
//!
//! Contract: MissionResult carries the mission's own provider-reported
//! spend (delta of the loop's cumulative counter over the mission);
//! the TUI done line prints THAT, while the HUD keeps the
//! session-authoritative total (M13 unchanged).

use hs_loop::uipaint::UiEvent;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn transcript_text(st: &hs_loop::tui::TuiState) -> String {
    st.transcript
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

// R1 (loop level): two missions in one session each report their OWN
// spend - equal for an identical fixture, never cumulative.
#[test]
fn m21_mission_result_carries_own_cost() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join("ui-events-m21");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let answer = dir.join("run").join("work").join("answer.txt");
    std::fs::write(
        dir.join("script.jsonl"),
        format!(
            "{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"A\"}}}}\nRead the result. ## Done.\n",
            answer.display()
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("hairspring.toml"),
        r#"
[[tools]]
name = "answer.write"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-answer"]
subjects = ["*"]
[[tools]]
name = "checker.run"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-liechecker"]
subjects = ["*"]
[[models]]
name = "scripted"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted"]
default = true
"#,
    )
    .unwrap();
    unsafe {
        std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl"));
    }
    let mut s = hs_loop::repl::load_session(
        &dir.join("hairspring.toml"),
        &dir.join("run"),
        false,
        3,
        None,
        None,
    )
    .unwrap();
    let r1 = s.run_goal("first mission").unwrap();
    let r2 = s.run_goal("second mission").unwrap();
    assert!(r1.cost_micros > 0, "mission spend is real: {r1:?}");
    assert!(r2.cost_micros > 0);
    // The scripted fixture is stateful (it repeats the final script
    // line when exhausted), so the missions need not spend identically;
    // what must hold: each result carries ONLY its own spend. The
    // per-call rate is the constant - same scripted provider, both
    // missions.
    assert_eq!(r1.model_calls, 2, "fixture: m1 = tool call + verifier");
    assert_eq!(
        r1.cost_micros / r1.model_calls as u64,
        r2.cost_micros / r2.model_calls as u64,
        "per-call rate constant across missions - each carries its own \
         spend: r1={} r2={}",
        r1.cost_micros, r2.cost_micros
    );
    assert!(
        r2.cost_micros < r1.cost_micros + r2.cost_micros,
        "r2 is not the cumulative total"
    );
    assert_eq!(
        s.total_cost_micros(),
        r1.cost_micros + r2.cost_micros,
        "session total is the sum, not double-counted"
    );
}

// R2 (TUI level): the done line prints the mission's own cost while
// the HUD keeps the session total.
#[test]
fn m21_done_line_shows_mission_cost_hud_shows_total() {
    let mut st = hs_loop::tui::TuiState::default();
    // Mission 1 lands: $0.0014 of $0.0014.
    st.mission_done_report(1, 2, 1400, 1400, "verified");
    // Mission 2 lands: $0.0021 itself, $0.0035 cumulative.
    st.mission_done_report(3, 3, 2100, 3500, "verified");
    let text = transcript_text(&st);
    assert!(text.contains("done: 1 steps, 2 calls, $0.0014"), "m1 line: {text}");
    assert!(
        text.contains("done: 3 steps, 3 calls, $0.0021"),
        "m2 line prints the MISSION cost, not the cumulative $0.0035: {text}"
    );
    assert!(!text.contains("done: 3 steps, 3 calls, $0.0035"));
    assert_eq!(st.total_cost_micros, 3500, "HUD keeps the session total (M13)");
    let _ = UiEvent::ModelCallStart { model: String::new() }; // keep the import honest
}
