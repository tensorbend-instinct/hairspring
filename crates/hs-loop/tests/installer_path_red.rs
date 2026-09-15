//! RED from the 2026-09-15 box install: the box's login PATH has no
//! ~/.local/bin, so install.sh linked the binary and then died on its own
//! resolution check, and even a manual install left
//! `bash -lc 'type -a hairspring'` resolving nothing - the exact "nothing
//! on PATH" failure Eric hit on 09-14. A fresh install must finish
//! successfully and leave exactly one normally resolvable hairspring
//! binary on a real login shell, without an ad hoc PATH. When the binlink
//! dir is missing from PATH the installer must add it to the login
//! profile with an idempotent, append-only managed block that preserves
//! unrelated profile content.
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn write_fake_rustup(bin: &std::path::Path) {
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
}

fn stage_repo(t: &tempfile::TempDir) -> std::path::PathBuf {
    let repo = t.path().join("repo");
    std::fs::create_dir_all(repo.join("examples")).unwrap();
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
    repo
}

#[test]
fn fresh_install_without_binlink_on_path_resolves_on_a_login_shell() {
    let t = tempfile::tempdir().unwrap();
    let repo = stage_repo(&t);
    let home = t.path().join("home");
    let cargo_home = t.path().join("cargo-home");
    let bin = t.path().join("fake-bin");
    let links = t.path().join("links");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    write_fake_rustup(&bin);
    // A stock profile with unrelated user content that must survive.
    std::fs::write(
        home.join(".profile"),
        "# ~/.profile\n# user custom line: keep me\n",
    )
    .unwrap();
    // The PATH deliberately EXCLUDES the binlink dir: the box defect.
    let path = format!("{}:/usr/bin:/bin", bin.display());
    let run_install = || {
        Command::new("sh")
            .arg("install.sh")
            .current_dir(&repo)
            .env("HOME", &home)
            .env("CARGO_HOME", &cargo_home)
            .env("PATH", &path)
            .env("HS_PREFIX", t.path().join("prefix"))
            .env("HS_BINLINK_DIR", &links)
            .output()
            .unwrap()
    };
    let out = run_install();
    assert!(
        out.status.success(),
        "install must succeed when the binlink dir is not on PATH: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // Unrelated profile content preserved; managed block added once.
    let profile = std::fs::read_to_string(home.join(".profile")).unwrap();
    assert!(
        profile.contains("# user custom line: keep me"),
        "unrelated profile content must survive: {profile}"
    );
    let marker = ">>> hairspring PATH (managed by install.sh) >>>";
    assert_eq!(
        profile.matches(marker).count(),
        1,
        "managed block added exactly once: {profile}"
    );
    // A real login shell resolves exactly one hairspring: ours.
    let login = Command::new("bash")
        .arg("-lc")
        .arg("type -a hairspring")
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    let listing = String::from_utf8_lossy(&login.stdout);
    let resolved: Vec<&str> = listing
        .lines()
        .filter(|l| l.contains("hairspring"))
        .collect();
    assert_eq!(
        resolved.len(),
        1,
        "login shell must resolve exactly one hairspring: {listing} stderr={}",
        String::from_utf8_lossy(&login.stderr)
    );
    assert!(
        resolved[0].contains(&links.join("hairspring").display().to_string()),
        "the resolved binary must be the installed link: {listing}"
    );
    // Reinstall is idempotent: no duplicate managed block.
    let out2 = run_install();
    assert!(
        out2.status.success(),
        "reinstall must succeed: stderr={}",
        String::from_utf8_lossy(&out2.stderr)
    );
    let profile2 = std::fs::read_to_string(home.join(".profile")).unwrap();
    assert_eq!(
        profile2.matches(marker).count(),
        1,
        "managed block must stay singular across reinstalls: {profile2}"
    );
}
