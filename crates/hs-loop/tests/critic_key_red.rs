//! Beat 6 RED (first-run blocker): the critic's key resolution must
//! mirror the operator model's (beat 3). `hairspring setup` persists
//! the key to ~/.config/hairspring/keys/<provider>.key, but the
//! critic read only HS_DEEPSEEK_API_KEY / HS_DEEPSEEK_API_KEY_FILE -
//! a stranger who completed guided setup and started a mission hit
//! "critic gate: neither HS_DEEPSEEK_API_KEY nor
//! HS_DEEPSEEK_API_KEY_FILE - fail-closed" on EVERY answer.submit
//! (live proof: caprun7 stream, 2026-09-10).

use hs_loop::critic::ProviderCritic;

// One test file = one process; the two tests mutate disjoint env
// states but cargo runs them in parallel threads, so serialize.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn critic_key_falls_back_to_config_dir() {
    let _g = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().unwrap();
    let keys = home.path().join(".config/hairspring/keys");
    std::fs::create_dir_all(&keys).unwrap();
    std::fs::write(keys.join("deepseek.key"), "sk-test-critic-fallback\n").unwrap();
    unsafe {
        std::env::remove_var("HS_DEEPSEEK_API_KEY");
        std::env::remove_var("HS_DEEPSEEK_API_KEY_FILE");
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::set_var("HOME", home.path());
    }
    ProviderCritic::for_provider(hs_loop::realmodel::deepseek())
        .expect("the setup-saved config-dir key must satisfy the critic");
}

#[test]
fn critic_error_names_setup_when_no_key() {
    let _g = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().unwrap();
    unsafe {
        std::env::remove_var("HS_DEEPSEEK_API_KEY");
        std::env::remove_var("HS_DEEPSEEK_API_KEY_FILE");
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::set_var("HOME", home.path());
    }
    let e = ProviderCritic::for_provider(hs_loop::realmodel::deepseek())
        .err()
        .expect("no key anywhere must still fail closed");
    assert!(
        e.contains("hairspring setup"),
        "the failure must name the fix, not just the missing vars: {e}"
    );
}
