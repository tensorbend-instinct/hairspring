//! RED (2026-09-07): the production SWE runner must build its kernel WITH a
//! log root, or every log-gated kernel feature is silently dead on the live
//! path. Live evidence: the 50f24656 audit run of conan-17302 emitted ZERO
//! dispatch-stage Observation records and no log/stderr/ directory, because
//! hs-swe-run.rs built its kernel with `Kernel::load` (no `log_root`) while
//! af5f7b57 gated dispatch records + stderr capture behind `load_with_log`.
//! Behavior proof of the features themselves: hs-kernel `wedge_visibility_red`.
//! This test pins the WIRING contract: the constructor the swe path uses
//! must hand the kernel a log root.

#[test]
fn swe_kernel_carries_a_log_root() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("hairspring.toml");
    std::fs::write(&cfg, "").unwrap();
    let log_root = dir.path().join("log");
    let k = hs_loop::swe_kernel(&cfg, &log_root).expect("swe kernel");
    assert!(
        k.has_log_root(),
        "swe-path kernel has no log root - dispatch records and stderr capture are dead"
    );
}

/// The harness must guard itself: a runner whose kernel lost its log root
/// (rewiring regression, wrong constructor) must REFUSE to start, loudly -
/// not run blind. RED: `require_visibility` does not exist yet.
#[test]
fn runner_without_visibility_wiring_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("hairspring.toml");
    std::fs::write(&cfg, "").unwrap();

    // the blind construction path (what hs-swe-run did before the fix)
    let blind = hs_kernel::Kernel::load(&cfg).expect("load");
    let refused = hs_loop::require_visibility(&blind);
    assert!(
        refused.is_err(),
        "a kernel with no log root must be refused, got {refused:?}"
    );
    let msg = refused.unwrap_err();
    assert!(
        msg.contains("log root") && msg.contains("visibility"),
        "refusal must name the defect plainly, got: {msg}"
    );

    // the wired path passes the same gate
    let log_root = dir.path().join("log");
    let wired = hs_loop::swe_kernel(&cfg, &log_root).expect("swe kernel");
    hs_loop::require_visibility(&wired).expect("wired kernel must pass the gate");
}
