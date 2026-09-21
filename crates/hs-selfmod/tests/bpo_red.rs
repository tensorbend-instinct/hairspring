use hs_selfmod::bpo::*;
#[test]
fn grouped_terminal_rewards_replace_a_critic() {
    let a = normalized_advantages(&[0.0, 1.0]);
    assert!((a[0] + 1.0).abs() < 1e-12);
    assert!((a[1] - 1.0).abs() < 1e-12);
    assert_eq!(normalized_advantages(&[1.0, 1.0]), vec![0.0, 0.0]);
}
#[test]
fn smoothed_complement_ratio_is_capped_and_directionally_masked() {
    let c = BpoConfig::default();
    let t = token_term(1.0, 0.9, 0.1, c).unwrap();
    assert!((t.mismatch_weight - 0.2).abs() < 1e-12);
    assert!(t.mask);
    let masked = token_term(-1.0, 0.9, 0.1, c).unwrap();
    assert!(!masked.mask);
    assert_eq!(masked.loss, 0.0);
    let mut c2 = c;
    c2.clip_high = 100.0;
    let capped = token_term(1.0, 0.0, 0.99, c2).unwrap();
    assert!(capped.mismatch_weight > c.cap);
    assert_eq!(capped.loss, -c.cap * 0.99_f64.ln());
}
