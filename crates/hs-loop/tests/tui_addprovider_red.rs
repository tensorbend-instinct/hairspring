//! RED: `/models add` + zero-config stranger path (Eric 2026-09-12:
//! "why isn't `OpenRouter` configurable from the TUI?" and "config should
//! not need to be specified - auto created and auto configured from the
//! TUI, same for --dir").
//!
//! The flow must land three artifacts with no hand-editing:
//! 1. a `[[providers]]` entry in `providers.toml` (the provmodel shape),
//! 2. a `[[models]]` block in the rig on the generic `hs-plugin-provmodel`
//!    plugin (so the kernel hot-reload lists it in `/models`),
//! 3. the API key through the same owner-only path as `hairspring setup`
//!    (`setup::save_key` - reused, already tested; not re-pinned here).
//!
//! Bare `hairspring` must work: no `--config` (rig auto-created from the
//! shipped template), no `--dir` (a session dir created where it belongs).
//! And `install.sh` must ship `hs-plugin-provmodel`, or the new model's
//! plugin binary does not exist on a fresh install.

use hs_loop::realmodel::load_providers_toml;
use hs_loop::repl::{
    AddProviderWizard, WizardFeed, add_provider, default_session_dir, resolve_config_path,
    resolve_session_dir,
};
use hs_loop::tui::EditorState;
use std::io::Write;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct EnvGuard(Vec<(&'static str, Option<String>)>);
impl EnvGuard {
    fn clear(keys: &[&'static str]) -> Self {
        let saved: Vec<(&'static str, Option<String>)> = keys
            .iter()
            .map(|k| {
                let old = std::env::var(k).ok();
                unsafe { std::env::remove_var(k) };
                (*k, old)
            })
            .collect();
        EnvGuard(saved)
    }

    fn set(pairs: &[(&'static str, &str)]) -> Self {
        let saved: Vec<(&'static str, Option<String>)> = pairs
            .iter()
            .map(|(k, v)| {
                let old = std::env::var(k).ok();
                unsafe { std::env::set_var(k, v) };
                (*k, old)
            })
            .collect();
        EnvGuard(saved)
    }
}
impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (k, old) in self.0.drain(..) {
            unsafe {
                match old {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }
}

fn rig_file() -> tempfile::NamedTempFile {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    write!(
        f,
        r#"[[models]]
name = "deepseek"
command = ["/opt/hs/bin/hs-plugin-deepseek"]
default = true
subjects = ["*"]

[[models]]
name = "scripted"
command = ["/opt/hs/bin/hs-plugin-scripted"]
subjects = ["*"]
"#
    )
    .unwrap();
    f
}

#[test]
fn add_provider_lands_providers_toml_and_rig_block() {
    let rig = rig_file();
    let dir = tempfile::tempdir().unwrap();
    let prov = dir.path().join("providers.toml");
    add_provider(
        rig.path(),
        &prov,
        "openrouter",
        "https://openrouter.ai/api/v1/chat/completions",
        "openai/gpt-5.2",
    )
    .unwrap();

    // 1. providers.toml parses through the production loader.
    let cfgs = load_providers_toml(&prov).unwrap();
    assert_eq!(cfgs.len(), 1);
    assert_eq!(cfgs[0].name, "openrouter");
    assert_eq!(
        cfgs[0].base_url,
        "https://openrouter.ai/api/v1/chat/completions"
    );
    assert_eq!(cfgs[0].model, "openai/gpt-5.2");

    // 2. the rig gains a [[models]] block on the generic plugin, the
    //    provmodel binary derived as a sibling of the existing plugins,
    //    and the existing blocks survive (default intact).
    let text = std::fs::read_to_string(rig.path()).unwrap();
    assert!(text.contains("[[models]]\nname = \"openrouter\""), "{text}");
    assert!(
        text.contains("command = [\"/opt/hs/bin/hs-plugin-provmodel\", \"openrouter\"]"),
        "{text}"
    );
    assert!(text.contains(
        "name = \"deepseek\"\ncommand = [\"/opt/hs/bin/hs-plugin-deepseek\"]\ndefault = true"
    ));
    let v: toml::Value = toml::from_str(&text).unwrap();
    assert_eq!(v.get("models").unwrap().as_array().unwrap().len(), 3);
}

#[test]
fn add_provider_creates_providers_toml_when_missing_and_appends() {
    let rig = rig_file();
    let dir = tempfile::tempdir().unwrap();
    let prov = dir.path().join("providers.toml");
    add_provider(rig.path(), &prov, "openrouter", "https://openrouter.ai/api/v1/chat/completions", "openai/gpt-5.2").unwrap();
    add_provider(rig.path(), &prov, "local", "http://localhost:8317/v1/chat/completions", "qwen3-coder").unwrap();
    let cfgs = load_providers_toml(&prov).unwrap();
    assert_eq!(cfgs.len(), 2);
    assert_eq!(cfgs[1].name, "local");
}

#[test]
fn add_provider_rejects_duplicates_and_bad_input() {
    let rig = rig_file();
    let dir = tempfile::tempdir().unwrap();
    let prov = dir.path().join("providers.toml");
    assert!(add_provider(rig.path(), &prov, "deepseek", "https://x.example/v1/chat/completions", "m").is_err());
    for bad in ["", "has space", "UPPER", "quo\"te", "-leading", "new\nline"] {
        assert!(
            add_provider(rig.path(), &prov, bad, "https://x.example/v1/chat/completions", "m").is_err(),
            "accepted bad name {bad:?}"
        );
    }
    for bad in ["", "ftp://x", "not-a-url", "https://"] {
        assert!(
            add_provider(rig.path(), &prov, "ok-name", bad, "m").is_err(),
            "accepted bad url {bad:?}"
        );
    }
    for bad in ["", "with\nnewline", "quo\"te"] {
        assert!(
            add_provider(rig.path(), &prov, "ok-name", "https://x.example/v1/chat/completions", bad).is_err(),
            "accepted bad model {bad:?}"
        );
    }
    assert!(!prov.exists(), "a rejected add leaves no partial providers.toml");
}

#[test]
fn wizard_walks_name_url_model_key_then_ready() {
    let mut w = AddProviderWizard::new();
    assert!(!w.is_secret());
    match w.feed("openrouter").unwrap() {
        WizardFeed::Next(p) => assert!(p.contains("base URL"), "{p}"),
        other => panic!("expected Next, got {other:?}"),
    }
    match w.feed("https://openrouter.ai/api/v1/chat/completions").unwrap() {
        WizardFeed::Next(p) => assert!(p.contains("model"), "{p}"),
        other => panic!("expected Next, got {other:?}"),
    }
    match w.feed("openai/gpt-5.2").unwrap() {
        WizardFeed::Next(p) => assert!(p.to_lowercase().contains("key"), "{p}"),
        other => panic!("expected Next, got {other:?}"),
    }
    assert!(w.is_secret(), "the key step hides input");
    match w.feed("sk-or-test-123").unwrap() {
        WizardFeed::Ready { name, base_url, model, key } => {
            assert_eq!(name, "openrouter");
            assert_eq!(base_url, "https://openrouter.ai/api/v1/chat/completions");
            assert_eq!(model, "openai/gpt-5.2");
            assert_eq!(key, "sk-or-test-123");
        }
        other => panic!("expected Ready, got {other:?}"),
    }
}

#[test]
fn wizard_cancel_and_bad_lines() {
    let mut w = AddProviderWizard::new();
    w.feed("openrouter").unwrap();
    assert!(matches!(w.feed("/cancel").unwrap(), WizardFeed::Cancelled));
    let mut w2 = AddProviderWizard::new();
    assert!(w2.feed("   ").is_err());
    assert!(w2.feed("bad name").is_err());
    assert!(matches!(w2.feed("ok-name").unwrap(), WizardFeed::Next(_)));
}

#[test]
fn secret_input_skips_history_and_masks_display() {
    let mut e = EditorState::default();
    e.set_text("sk-secret");
    e.set_secret(true);
    assert_eq!(e.display_text(), "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}");
    let submitted = e.submit().unwrap();
    assert_eq!(submitted, "sk-secret");
    assert!(e.history_entries().is_empty(), "secret never enters history");
    e.set_secret(false);
    e.set_text("visible goal");
    e.submit().unwrap();
    assert_eq!(e.history_entries(), vec!["visible goal".to_string()]);
    assert_eq!(e.display_text(), "");
}

#[test]
fn bare_invocation_resolves_config_and_dir() {
    let _l = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let xdg_cfg = tmp.path().join("cfg");
    let xdg_data = tmp.path().join("data");
    let home = tmp.path().join("home");
    let _g = EnvGuard::set(&[
        ("XDG_CONFIG_HOME", xdg_cfg.to_str().unwrap()),
        ("XDG_DATA_HOME", xdg_data.to_str().unwrap()),
        ("HOME", home.to_str().unwrap()),
    ]);

    // no --config: the rig is auto-created from the shipped template,
    // @PREFIX@ resolved, under the XDG config dir
    let cfg = resolve_config_path(None).unwrap();
    assert_eq!(cfg, xdg_cfg.join("hairspring").join("hairspring.toml"));
    assert!(cfg.is_file(), "rig auto-created");
    let text = std::fs::read_to_string(&cfg).unwrap();
    assert!(text.contains("[[models]]"), "{text}");
    assert!(!text.contains("@PREFIX@"), "template tokens resolved");
    // a second call keeps the existing rig (never clobbers)
    std::fs::write(&cfg, "# mine\n").unwrap();
    let cfg2 = resolve_config_path(None).unwrap();
    assert_eq!(std::fs::read_to_string(&cfg2).unwrap(), "# mine\n");

    // no --dir: a session dir under the XDG data dir, created
    let d = resolve_session_dir(None).unwrap();
    assert_eq!(d, xdg_data.join("hairspring").join("run"), "{d:?}");
    assert!(d.is_dir(), "session dir auto-created");
    assert_eq!(default_session_dir(), d);

    // explicit flags still win
    let explicit = resolve_config_path(Some("/tmp/explicit.toml")).unwrap();
    assert_eq!(explicit, std::path::PathBuf::from("/tmp/explicit.toml"));
    let explicit_d = resolve_session_dir(Some("/tmp/explicit-dir")).unwrap();
    assert_eq!(explicit_d, std::path::PathBuf::from("/tmp/explicit-dir"));
}

#[test]
fn default_session_dir_falls_back_to_home_local_share() {
    let _l = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home2");
    let saved: Vec<(&'static str, Option<String>)> = ["XDG_DATA_HOME", "HOME"]
        .iter()
        .map(|k| {
            let old = std::env::var(k).ok();
            unsafe { std::env::remove_var(k) };
            (*k, old)
        })
        .collect();
    unsafe { std::env::set_var("HOME", home.to_str().unwrap()) };
    let d = default_session_dir();
    assert_eq!(d, home.join(".local").join("share").join("hairspring").join("run"));
    for (k, old) in saved {
        unsafe {
            match old {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }
}

#[test]
fn install_ships_provmodel_plugin() {
    let install = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../install.sh")).unwrap();
    assert!(
        install.contains("hs-plugin-provmodel"),
        "install.sh must ship hs-plugin-provmodel - a TUI-added provider points at it"
    );
}

#[test]
fn kernel_reload_after_add_sees_the_model() {
    // The /models pick must work without a restart: the session reloads
    // the kernel after the write, and the new block resolves on the
    // generic plugin (the binaries sit side by side in the rig).
    let _l = ENV_LOCK.lock().unwrap();
    let _k = EnvGuard::set(&[
        ("HS_OPENROUTER_API_KEY", "sk-or-test"),
        ("HS_DEEPSEEK_API_KEY", "sk-test"),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let ds = env!("CARGO_BIN_EXE_hs-plugin-deepseek");
    let rig = dir.path().join("rig.toml");
    std::fs::write(
        &rig,
        format!(
            "[[models]]\nname = \"deepseek\"\ncommand = [\"{ds}\"]\ndefault = true\nsubjects = [\"*\"]\n"
        ),
    )
    .unwrap();
    let prov = dir.path().join("providers.toml");
    let mut k = hs_kernel::Kernel::load(&rig).unwrap();
    assert!(k.has_model("deepseek"));
    assert!(!k.has_model("openrouter"));
    add_provider(
        &rig,
        &prov,
        "openrouter",
        "https://openrouter.ai/api/v1/chat/completions",
        "openai/gpt-5.2",
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(25)); // mtime tick
    assert!(k.reload_if_changed().unwrap(), "reload not detected");
    assert!(k.has_model("openrouter"), "the added model serves");
}

#[test]
fn tui_launch_opens_without_a_credential() {
    // Zero-config stranger path (Eric 2026-09-12): a first-run TUI with
    // no key must OPEN - /models add is the fix and it lives in the TUI.
    // A one-shot mission keeps the hard gate (never a mid-mission 400).
    let _l = ENV_LOCK.lock().unwrap();
    let _k = EnvGuard::clear(&["HS_DEEPSEEK_API_KEY", "HS_DEEPSEEK_API_KEY_FILE"]);
    let dir = tempfile::tempdir().unwrap();
    let home: &'static str = Box::leak(dir.path().display().to_string().into_boxed_str());
    let xdg: &'static str =
        Box::leak(dir.path().join("xdg").display().to_string().into_boxed_str());
    let _h = EnvGuard::set(&[("HOME", home), ("XDG_CONFIG_HOME", xdg)]);
    let ds = env!("CARGO_BIN_EXE_hs-plugin-deepseek");
    let rig = dir.path().join("rig.toml");
    std::fs::write(
        &rig,
        format!(
            "[[models]]\nname = \"deepseek\"\ncommand = [\"{ds}\"]\ndefault = true\nsubjects = [\"*\"]\n"
        ),
    )
    .unwrap();
    hs_loop::setup::readiness_gate(&rig, true, true)
        .expect("the TUI launch opens: /models add fixes the credential there");
    assert!(hs_loop::setup::readiness_gate(&rig, false, false).is_err());
}

#[test]
fn tui_session_loads_with_an_uncredentialed_default() {
    // Zero-config stranger path (Eric 2026-09-12): the first-run TUI
    // must OPEN even when the default model has no key - /models add is
    // the in-place fix. The strict load keeps refusing (one-shot runs
    // never burn a mission on a mid-mission 400).
    let _l = ENV_LOCK.lock().unwrap();
    let _k = EnvGuard::clear(&["HS_DEEPSEEK_API_KEY", "HS_DEEPSEEK_API_KEY_FILE"]);
    let dir = tempfile::tempdir().unwrap();
    let home: &'static str = Box::leak(dir.path().display().to_string().into_boxed_str());
    let xdg: &'static str =
        Box::leak(dir.path().join("xdg").display().to_string().into_boxed_str());
    let _h = EnvGuard::set(&[("HOME", home), ("XDG_CONFIG_HOME", xdg)]);
    let ds = env!("CARGO_BIN_EXE_hs-plugin-deepseek");
    let rig = dir.path().join("rig.toml");
    std::fs::write(
        &rig,
        format!(
            "[[models]]\nname = \"deepseek\"\ncommand = [\"{ds}\"]\ndefault = true\nsubjects = [\"*\"]\n"
        ),
    )
    .unwrap();
    assert!(hs_kernel::Kernel::load_with_log(&rig, dir.path()).is_err());
    let k = hs_kernel::Kernel::load_lenient(&rig, dir.path())
        .expect("the TUI session loads: /models add fixes the credential in place");
    assert!(k.has_model("deepseek"));
}
