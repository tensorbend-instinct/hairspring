//! Eric 2026-09-10 (iMessage): "it can be installed if needed - doesn't
//! need to be pre installed." The CAPABILITY is the contract: a confined
//! mission can download and install the toolchain it needs (node, go,
//! rust/cargo) into its workspace or blessed writable dirs and use it -
//! zero system-wide writes. Each test below bootstraps one toolchain
//! from scratch INSIDE the confinement and runs something real.
//! Own binary + serial env lock: HS_PROJECT_ROOT is process-global.
//! Slow by nature (real vendor downloads); run with a generous timeout.

use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct RootGuard(std::path::PathBuf);
impl RootGuard {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("hs-boot-{tag}-{}", std::process::id()));
        let task = root.join("task");
        std::fs::create_dir_all(&task).unwrap();
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

fn run_ok(task: &std::path::Path, cmd: &str, secs: u64) -> String {
    let r = hs_loop::termexec::run(task, cmd, secs);
    let out = format!("{}{}", r["stdout"].as_str().unwrap(), r["stderr"].as_str().unwrap());
    assert_eq!(r["exit_code"], 0, "exit nonzero: {out}");
    out
}

#[test]
fn confined_bootstraps_node() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let guard = RootGuard::new("node");
    let out = run_ok(&guard.0, r#"
        set -e
        TARBALL=$(curl -fsSL https://nodejs.org/dist/latest-v22.x/ | grep -o 'node-v[0-9.]*-linux-x64\.tar\.gz' | head -1)
        test -n "$TARBALL"
        curl -fsSL -o "$TARBALL" "https://nodejs.org/dist/latest-v22.x/$TARBALL"
        curl -fsSL -o SHASUMS256.txt "https://nodejs.org/dist/latest-v22.x/SHASUMS256.txt"
        grep "$TARBALL" SHASUMS256.txt | sha256sum -c -
        tar --no-same-owner -xzf "$TARBALL"
        NODEDIR=$(echo "$TARBALL" | sed 's/\.tar\.gz$//')
        export PATH="$PWD/$NODEDIR/bin:$PATH"
        "./$NODEDIR/bin/node" --version
        "./$NODEDIR/bin/node" -e 'const fs=require("fs");fs.writeFileSync("answer-node.txt","node-boot-ok "+process.version);console.log("NODE-BOOT-OK",process.version)'
        "./$NODEDIR/bin/npm" --version
    "#, 420);
    assert!(out.contains("NODE-BOOT-OK"), "{out}");
    assert!(out.contains(": OK"), "sha256 verified: {out}");
}

#[test]
fn confined_bootstraps_go() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let guard = RootGuard::new("go");
    let out = run_ok(&guard.0, r#"
        set -e
        TARBALL=$(curl -fsSL 'https://go.dev/dl/?mode=json' | grep -o 'go[0-9][0-9.]*\.linux-amd64\.tar\.gz' | head -1)
        test -n "$TARBALL"
        curl -fsSL -o go.tar.gz "https://go.dev/dl/$TARBALL"
        tar --no-same-owner -xzf go.tar.gz
        export GOROOT="$PWD/go" GOPATH="$PWD/gopath" GOCACHE="$PWD/gocache" PATH="$PWD/go/bin:$PATH"
        go version
        mkdir -p hello && cd hello
        printf 'package main\nimport "fmt"\nfunc main() { fmt.Println("GO-BOOT-OK") }\n' > main.go
        go mod init hello
        go build -o hello .
        ./hello
    "#, 420);
    assert!(out.contains("GO-BOOT-OK"), "{out}");
}

#[test]
fn confined_bootstraps_rust() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let guard = RootGuard::new("rust");
    let out = run_ok(&guard.0, r#"
        set -e
        curl -fsSL -o rustup-init https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init
        chmod +x rustup-init
        export CARGO_HOME="$PWD/.cargo" RUSTUP_HOME="$PWD/.rustup" PATH="$PWD/.cargo/bin:$PATH"
        ./rustup-init -y --profile minimal --default-toolchain stable --no-modify-path
        cargo --version
        cargo init hello --name hello
        cd hello
        printf 'fn main() { println!("RUST-BOOT-OK"); }\n' > src/main.rs
        cargo build
        ./target/debug/hello
    "#, 600);
    assert!(out.contains("RUST-BOOT-OK"), "{out}");
}
