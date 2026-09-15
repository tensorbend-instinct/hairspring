//! RED from the 2026-09-15 clean-HOME install: rustup honored an explicit
//! CARGO_HOME, then install.sh sourced $HOME/.cargo/env and died before build.
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn fresh_install_sources_the_rustup_env_from_cargo_home() {
    let t = tempfile::tempdir().unwrap();
    let repo = t.path().join("repo");
    let home = t.path().join("home");
    let cargo_home = t.path().join("cargo-home");
    let bin = t.path().join("fake-bin");
    std::fs::create_dir_all(repo.join("examples")).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    for f in ["install.sh", "hairspring.example.toml"] {
        std::fs::copy(
            format!("{}/../../{f}", env!("CARGO_MANIFEST_DIR")),
            repo.join(f),
        )
        .unwrap();
    }
    std::fs::copy(
        format!(
            "{}/../../examples/seqmodel-demo.jsonl",
            env!("CARGO_MANIFEST_DIR")
        ),
        repo.join("examples/seqmodel-demo.jsonl"),
    )
    .unwrap();
    let curl = bin.join("curl");
    std::fs::write(&curl, r#"#!/bin/sh
cat <<'INSTALLER'
#!/bin/sh
set -eu
mkdir -p "$CARGO_HOME/bin"
cat > "$CARGO_HOME/bin/cargo" <<'CARGO'
#!/bin/sh
set -eu
mkdir -p "$CARGO_TARGET_DIR/release"
for b in hs-repl hs-log-cli hs-plugin-answer hs-plugin-answersubmit hs-plugin-selfcheck hs-plugin-critic hs-plugin-fileread hs-plugin-reposearch hs-plugin-repoexec hs-plugin-editapply hs-plugin-notescratch hs-plugin-termexec hs-plugin-swarm hs-plugin-policy hs-plugin-scripted hs-plugin-deepseek hs-plugin-provmodel hs-promote hs-plugin-gatemodel hs-plugin-checker; do
  printf '#!/bin/sh\n' > "$CARGO_TARGET_DIR/release/$b"
  chmod +x "$CARGO_TARGET_DIR/release/$b"
done
CARGO
chmod +x "$CARGO_HOME/bin/cargo"
printf 'export PATH="%s/bin:$PATH"\n' "$CARGO_HOME" > "$CARGO_HOME/env"
INSTALLER
"#).unwrap();
    let mut mode = std::fs::metadata(&curl).unwrap().permissions();
    mode.set_mode(0o755);
    std::fs::set_permissions(&curl, mode).unwrap();
    let out = Command::new("sh")
        .arg("install.sh")
        .current_dir(&repo)
        .env("HOME", &home)
        .env("CARGO_HOME", &cargo_home)
        .env(
            "PATH",
            format!(
                "{}:{}:/usr/bin:/bin",
                t.path().join("links").display(),
                bin.display()
            ),
        )
        .env("HS_PREFIX", t.path().join("prefix"))
        .env("HS_BINLINK_DIR", t.path().join("links"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "clean install must source CARGO_HOME/env: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
