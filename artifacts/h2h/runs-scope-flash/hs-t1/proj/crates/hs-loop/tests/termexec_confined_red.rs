//! RED: the CONFINED mission exec surface (HS_PROJECT_ROOT set) must let a
//! mission build language environments inside its project directory
//! (Eric 2026-09-10, iMessage: uv/venv + equivalents, blessed caches),
//! while PATH exposes the toolchain bins and DNS resolves.
//! Own binary: HS_PROJECT_ROOT is process-global.

use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct RootGuard(std::path::PathBuf);
impl RootGuard {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("hs-confined-{}", std::process::id()));
        let task = root.join("task");
        std::fs::create_dir_all(&root.join("task")).unwrap();
        unsafe {
            std::env::set_var("HS_PROJECT_ROOT", &root);
        }
        RootGuard(task)
    }
}
impl Drop for RootGuard {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var("HS_PROJECT_ROOT");
        }
    }
}

#[test]
fn confined_uv_venv_works() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let guard = RootGuard::new();
    let r = hs_loop::termexec::run(
        &guard.0,
        r#"uv venv .venv && uv pip install --python .venv/bin/python --quiet six && .venv/bin/python -c 'import six; print("MISSION-UV-OK")'"#,
        120,
    );
    let out = format!("{}{}", r["stdout"].as_str().unwrap(), r["stderr"].as_str().unwrap());
    assert_eq!(r["exit_code"], 0, "{out}");
    assert!(out.contains("MISSION-UV-OK"), "{out}");
}

#[test]
fn confined_path_and_dns() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let guard = RootGuard::new();
    let r = hs_loop::termexec::run(&guard.0, "command -v uv; echo PATH=$PATH; getent hosts pypi.org", 30);
    let out = format!("{}{}", r["stdout"].as_str().unwrap(), r["stderr"].as_str().unwrap());
    assert!(out.contains("/usr/local/bin/uv"), "uv on the confined PATH: {out}");
    assert!(out.contains("pypi.org"), "DNS resolves confined: {out}");
    assert_eq!(r["exit_code"], 0, "{out}");
}

#[test]
fn confined_writes_outside_root_denied() {
    // The ruling's hard boundary on the mission surface: the project root
    // is the only writable host filesystem.
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let guard = RootGuard::new();
    let r = hs_loop::termexec::run(
        &guard.0,
        "touch /usr/local/hs-termexec-escape 2>&1; echo touch_exit=$?; touch /ws 2>/dev/null; true",
        30,
    );
    let out = format!("{}{}", r["stdout"].as_str().unwrap(), r["stderr"].as_str().unwrap());
    assert!(
        out.contains("touch_exit=1") || out.contains("Read-only"),
        "writes outside the root fail: {out}"
    );
    assert!(
        !std::path::Path::new("/usr/local/hs-termexec-escape").exists(),
        "no host escape: {out}"
    );
}
