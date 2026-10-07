//! RED (2026-10-06): when a declared check fails, the feedback must say
//! whether the CAUSE is the environment (command not found) rather than
//! the code, and a verbatim-repeated failure must be called out so the
//! model stops resubmitting an unchanged failure ("doom loop"). Neither
//! softens the verdict: a failing check stays failed.

static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn task(name: &str, checks: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("hs-sc-fb-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let t = root.join("task");
    std::fs::create_dir_all(t.join(".hs")).unwrap();
    std::fs::write(t.join(".hs/checks"), checks).unwrap();
    unsafe {
        std::env::set_var("HS_PROJECT_ROOT", &root);
        std::env::set_var("HS_SELFCHECK_DIRECT", "1");
    }
    t
}

#[test]
fn command_not_found_is_labelled_environment_and_still_fails() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let t = task("env", "no-such-tool-xyz --version\n");
    let v = hs_loop::selfcheck::check(&t);
    assert_eq!(v["passed"], false);
    let e = v["error"].as_str().unwrap();
    assert!(e.contains("ENVIRONMENT"), "env class named: {e}");
    assert!(e.contains("command -v"), "tells the model how to look: {e}");
}

#[test]
fn ordinary_test_failure_is_not_labelled_environment() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let t = task("code", "echo 'assert 1 == 2'; exit 1\n");
    let v = hs_loop::selfcheck::check(&t);
    let e = v["error"].as_str().unwrap();
    assert!(!e.contains("ENVIRONMENT"), "{e}");
    assert!(!e.contains("REPEAT"), "first failure is not a repeat: {e}");
}

#[test]
fn identical_failure_repeats_are_counted_and_called_out() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let t = task("rep", "echo same; exit 1\n");
    let a = hs_loop::selfcheck::check(&t)["error"].as_str().unwrap().to_string();
    assert!(!a.contains("REPEAT"));
    let b = hs_loop::selfcheck::check(&t)["error"].as_str().unwrap().to_string();
    assert!(b.contains("REPEAT 2"), "{b}");
    let c = hs_loop::selfcheck::check(&t)["error"].as_str().unwrap().to_string();
    assert!(c.contains("REPEAT 3"), "{c}");
    // a changed failure resets the counter
    std::fs::write(t.join(".hs/checks"), "echo different; exit 1\n").unwrap();
    let d = hs_loop::selfcheck::check(&t)["error"].as_str().unwrap().to_string();
    assert!(!d.contains("REPEAT"), "{d}");
}

#[test]
fn a_pass_clears_the_repeat_counter() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let t = task("clr", "echo same; exit 1\n");
    let _ = hs_loop::selfcheck::check(&t);
    let _ = hs_loop::selfcheck::check(&t);
    std::fs::write(t.join(".hs/checks"), "true\n").unwrap();
    assert_eq!(hs_loop::selfcheck::check(&t)["passed"], true);
    std::fs::write(t.join(".hs/checks"), "echo same; exit 1\n").unwrap();
    let e = hs_loop::selfcheck::check(&t)["error"].as_str().unwrap().to_string();
    assert!(!e.contains("REPEAT"), "{e}");
}
