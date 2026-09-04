//! Live provider smoke test (seam gap C1): the realmodel adapter <-> real
//! provider <-> relay seam had only mock-server coverage, which is exactly
//! where the 2026-09-04 gzip/encoding bug escaped (ureq advertised gzip,
//! relay stripped Accept-Encoding silently). This test hits the REAL
//! provider through the REAL configured endpoint with compression offered,
//! and asserts a decodable completion comes back.
//!
//! Gated: runs only when HS_LIVE_SMOKE=1 (costs one real completion,
//! ~$0.001 metered). Requires the GLM key via HS_GLM_API_KEY or
//! HS_GLM_API_KEY_FILE and a reachable HS_GLM_BASE_URL (or the default).
//! Never runs in the default suite.

#[test]
fn live_glm_round_trip() {
    if std::env::var("HS_LIVE_SMOKE").as_deref() != Ok("1") {
        eprintln!("skipped: set HS_LIVE_SMOKE=1 to run the live smoke test");
        return;
    }
    let v = hs_loop::realmodel::call(
        &hs_loop::realmodel::glm(),
        "Reply with exactly this JSON object and nothing else: {\"pong\": true}",
    )
    .expect("live call must succeed - a failure here is the seam, not the model");
    let completion = v["completion"].as_str().unwrap_or("");
    assert!(
        completion.contains("pong"),
        "undecodable/garbled completion (gzip regression shape): {:?}",
        &completion[..completion.len().min(80)]
    );
    assert!(v["output_tokens"].as_u64().unwrap_or(0) > 0);
    eprintln!(
        "live smoke ok: {} out-tokens, {} micros",
        v["output_tokens"], v["cost_usd_micros"]
    );
}
