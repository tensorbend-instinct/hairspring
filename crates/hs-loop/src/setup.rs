//! Guided first-run setup (the Codex/Claude/exo bar, stranger burn
//! 2026-09-10): a stranger installs, runs `hairspring setup`, and gets a
//! working credential + rig without editing files or exporting env vars.
//! Key material lands in the config dir as an owner-only file - never in
//! the repo, never inline in the rig toml, never echoed.

use crate::realmodel::{self, Provider};
use std::path::PathBuf;

/// The config dir install.sh also uses: $XDG_CONFIG_HOME/hairspring or
/// ~/.config/hairspring.
pub fn config_dir() -> PathBuf {
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        if !x.is_empty() {
            return PathBuf::from(x).join("hairspring");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config").join("hairspring")
}

/// Where a provider's persisted key lives (owner-only file). The mission
/// path's `load_key` checks this exact path as its final fallback, so a
/// fresh shell needs no exports.
pub fn key_path(provider: &str) -> PathBuf {
    config_dir().join("keys").join(format!("{provider}.key"))
}

fn providers() -> Vec<Provider> {
    vec![realmodel::deepseek(), realmodel::glm()]
}

pub struct Readiness {
    pub provider: String,
    pub key_env: String,
    pub ready: bool,
}

pub fn check_readiness() -> Vec<Readiness> {
    providers()
        .iter()
        .map(|p| Readiness {
            provider: p.name.clone(),
            key_env: p.key_env.clone(),
            ready: realmodel::load_key(p).is_ok(),
        })
        .collect()
}

pub enum Validation {
    Valid,
    Rejected(String),
    Unknown(String),
}

/// One zero-cost round-trip: GET <base>/models with the fresh key. 2xx =>
/// valid; 401/403 => rejected (do not save); anything else (offline,
/// endpoint shape unknown) => Unknown - the operator is told and the key
/// is still saved.
pub fn validate_live(provider: &str, key: &str) -> Validation {
    let Some(p) = providers().into_iter().find(|p| p.name == provider) else {
        return Validation::Unknown(format!("unknown provider {provider}"));
    };
    let base = std::env::var(&p.base_url_env).unwrap_or_else(|_| p.default_base_url.clone());
    let root = base
        .strip_suffix("chat/completions")
        .unwrap_or(&base)
        .trim_end_matches('/');
    let url = format!("{root}/models");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(10)))
        // error statuses arrive as responses: 401/403 is the rejection
        // signal, not an exception
        .http_status_as_error(false)
        .build()
        .into();
    match agent
        .get(&url)
        .header("Authorization", &format!("Bearer {key}"))
        .call()
    {
        Ok(resp) => match resp.status().as_u16() {
            200..=299 => Validation::Valid,
            401 | 403 => Validation::Rejected(format!("HTTP {}", resp.status().as_u16())),
            other => Validation::Unknown(format!("HTTP {other} from {url}")),
        },
        Err(e) => Validation::Unknown(format!("{e}")),
    }
}

/// Persist a key owner-only (0600; the keys dir 0700). Returns the path.
pub fn save_key(provider: &str, key: &str) -> Result<PathBuf, String> {
    if !providers().iter().any(|p| p.name == provider) {
        return Err(format!(
            "unknown provider \"{provider}\" - known: deepseek, glm"
        ));
    }
    let key = key.trim();
    if key.is_empty() {
        return Err("empty key".into());
    }
    if key.chars().any(char::is_whitespace) {
        return Err("the key contains whitespace - paste it exactly as issued".into());
    }
    let path = key_path(provider);
    let dir = path.parent().expect("key path has a parent");
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    use std::io::Write as _;
    let mut f = opts
        .open(&path)
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    writeln!(f, "{key}").map_err(|e| format!("write {}: {e}", path.display()))?;
    drop(f);
    #[cfg(unix)]
    {
        // a pre-existing wider-permission file is tightened too
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(path)
}

/// Write the rig config from the shipped template when absent (never
/// clobber an existing rig - install.sh or the operator owns it).
/// @PREFIX@ resolves from the running binary's install location.
pub fn ensure_config() -> Result<PathBuf, String> {
    let cfg = config_dir().join("hairspring.toml");
    if cfg.exists() {
        return Ok(cfg);
    }
    let template = include_str!("../../../hairspring.example.toml");
    let exe = std::env::current_exe().map_err(|e| format!("current exe: {e}"))?;
    // The rig references plugins as @PREFIX@/bin/<plugin> - i.e. NEXT TO
    // the hairspring binary. Map @PREFIX@/bin to the running binary's own
    // directory so both layouts resolve: installed (<prefix>/bin/hs-repl)
    // and dev (target/<profile>/hs-repl, binaries alongside).
    let bin_dir = exe
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or("cannot resolve the binary's directory")?;
    let prefix = bin_dir.parent().map(|p| p.to_path_buf());
    let mut rendered = template.replace("@PREFIX@/bin", &bin_dir.to_string_lossy());
    if let Some(prefix) = prefix {
        rendered = rendered.replace("@PREFIX@", &prefix.to_string_lossy());
    }
    if let Some(parent) = cfg.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    std::fs::write(&cfg, rendered).map_err(|e| format!("write {}: {e}", cfg.display()))?;
    Ok(cfg)
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// The `hairspring setup` entry. `--check` prints per-provider readiness
/// (exit 1 when none ready). `--provider <name> --key-stdin` is the
/// scriptable save path. Bare on a TTY runs the wizard.
pub fn cli(args: &[String]) -> Result<(), String> {
    use std::io::IsTerminal as _;
    if args.iter().any(|a| a == "--check") {
        let mut any = false;
        for r in check_readiness() {
            println!(
                "{}: {}",
                r.provider,
                if r.ready { "ready" } else { "no credential" }
            );
            any |= r.ready;
        }
        if !any {
            println!("run `hairspring setup` to configure a provider");
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.iter().any(|a| a == "--key-stdin") {
        let provider = flag(args, "--provider").ok_or("--key-stdin needs --provider <name>")?;
        let mut key = String::new();
        use std::io::BufRead as _;
        std::io::stdin()
            .lock()
            .read_line(&mut key)
            .map_err(|e| format!("read key: {e}"))?;
        return finish_save(&provider, key.trim());
    }
    if !std::io::stdin().is_terminal() {
        return Err(
            "setup is interactive; non-interactive: `hairspring setup --provider <name> --key-stdin`, or `--check`"
                .into(),
        );
    }
    wizard()
}

fn finish_save(provider: &str, key: &str) -> Result<(), String> {
    match validate_live(provider, key) {
        Validation::Valid => eprintln!("key validated against the provider"),
        Validation::Rejected(e) => {
            return Err(format!("the provider rejected this key ({e}) - nothing saved"));
        }
        Validation::Unknown(e) => {
            eprintln!("could not validate the key ({e}) - saving anyway");
        }
    }
    let path = save_key(provider, key)?;
    println!("saved {provider} credential to {} (owner-only)", path.display());
    let cfg = ensure_config()?;
    println!("rig config: {}", cfg.display());
    println!(
        "next: hairspring run --goal \"write hello.txt containing hello\" --config {} --dir /tmp/hs-demo",
        cfg.display()
    );
    Ok(())
}

fn prompt(label: &str) -> Result<String, String> {
    use std::io::Write as _;
    eprint!("{label}");
    std::io::stderr().flush().map_err(|e| e.to_string())?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).map_err(|e| e.to_string())?;
    Ok(line.trim().to_string())
}

/// No-echo secret input (crossterm raw mode): the key never touches the
/// scrollback. Ctrl+C exits 130 like any other CLI.
fn read_secret(label: &str) -> Result<String, String> {
    use crossterm::event::{Event, KeyCode, KeyModifiers};
    use std::io::Write as _;
    eprint!("{label}");
    std::io::stderr().flush().map_err(|e| e.to_string())?;
    crossterm::terminal::enable_raw_mode().map_err(|e| format!("raw mode: {e}"))?;
    let mut buf = String::new();
    loop {
        match crossterm::event::read() {
            Ok(Event::Key(k)) => match k.code {
                KeyCode::Enter => break,
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    let _ = crossterm::terminal::disable_raw_mode();
                    eprintln!();
                    std::process::exit(130);
                }
                KeyCode::Backspace => {
                    buf.pop();
                }
                KeyCode::Char(c) => buf.push(c),
                _ => {}
            },
            Ok(_) => {}
            Err(e) => {
                let _ = crossterm::terminal::disable_raw_mode();
                return Err(format!("read key: {e}"));
            }
        }
    }
    let _ = crossterm::terminal::disable_raw_mode();
    eprintln!();
    Ok(buf)
}

fn wizard() -> Result<(), String> {
    println!("hairspring setup");
    println!();
    for r in check_readiness() {
        println!(
            "  {} {} ({})",
            if r.ready { "ready   " } else { "no cred " },
            r.provider,
            r.key_env
        );
    }
    println!();
    let pick = prompt("provider to configure [deepseek]: ")?;
    let provider = if pick.is_empty() { "deepseek" } else { pick.as_str() };
    let Some(p) = providers().into_iter().find(|p| p.name == provider) else {
        return Err(format!("unknown provider \"{provider}\" - known: deepseek, glm"));
    };
    let key = match std::env::var(&p.key_env) {
        Ok(k) if !k.trim().is_empty() => {
            let yn = prompt(&format!(
                "{} is already set in this environment - persist it? [Y/n]: ",
                p.key_env
            ))?;
            if yn.is_empty() || yn.eq_ignore_ascii_case("y") || yn.eq_ignore_ascii_case("yes") {
                k.trim().to_string()
            } else {
                read_secret(&format!("paste your {provider} API key (input hidden): "))?
            }
        }
        _ => read_secret(&format!("paste your {provider} API key (input hidden): "))?,
    };
    finish_save(provider, &key)
}
