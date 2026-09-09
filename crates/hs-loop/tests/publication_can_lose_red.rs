//! CAN-LOSE efficacy pin for the gate-8 publication rig (Eric's order:
//! "the harness must be able to LOSE - a rig that always shows a win is
//! reward hacking"). Two proofs through the real machinery:
//!
//! c1 (counter-arm): strengthen the incumbent to a fair race - zero
//! induced failed attempts means no doom loop for the feedback channel
//! to repair, so the honest verdict is LOSE, and the same
//! `run_publication` that printed WIN on the B10 run MUST print LOSE.
//!
//! c2 (render honesty): a `Publication` carrying `Verdict::Lose`
//! renders "verdict=LOSE" verbatim, never a dressed-up win.

use hs_loop::publication::{run_publication, ArmReport, Publication, Verdict};

#[test]
fn c1_strengthened_incumbent_flips_the_verdict_to_lose() {
    let dir = std::env::temp_dir().join("publication-can-lose-c1");
    let _ = std::fs::remove_dir_all(&dir);
    // Zero failed attempts: the incumbent passes first try with nothing
    // to repair - hairspring's feedback advantage is genuinely disabled
    // because there is no failure for it to convert. Both arms pass;
    // the verdict must still flip.
    let p = run_publication(&dir, 1, 0).unwrap();
    assert_eq!(p.incumbent.passed, 1, "incumbent arm: {p:?}");
    assert_eq!(p.hairspring.passed, 1, "hairspring arm: {p:?}");
    assert!(
        p.speedup_steps < 2.0,
        "no doom loop, no 2x: speedup={}",
        p.speedup_steps
    );
    assert_eq!(
        p.verdict,
        Verdict::Lose,
        "the rig MUST be able to lose: {p:?}"
    );
    let path = p.write(&dir.join("artifact")).unwrap();
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(
        body.contains("verdict=LOSE"),
        "the artifact prints the loss verbatim:\n{body}"
    );
}

#[test]
fn c2_lose_verdict_renders_verbatim_in_report_lines() {
    let p = Publication {
        suite: "token-family".to_string(),
        task_count: 1,
        failed_attempts: 0,
        incumbent: ArmReport {
            name: "incumbent".to_string(),
            ..ArmReport::default()
        },
        hairspring: ArmReport {
            name: "hairspring".to_string(),
            ..ArmReport::default()
        },
        speedup_steps: 0.5,
        verdict: Verdict::Lose,
    };
    let body = p.report_lines().join("\n");
    assert!(body.contains("verdict=LOSE"), "report body:\n{body}");
    assert!(
        !body.contains("verdict=WIN"),
        "a loss never renders as a win:\n{body}"
    );
}
