//! RED (2026-09-06): Eric's network policy - internet ON, but NOTHING inside
//! the mission environment may carry the gold answer or harness internals.
//! Reward hacking = reading the solution out of the eval environment.
//! This suite probes the REAL sandbox: from inside bwrap, every path that
//! could hold reference material (run_dir with test_patch.diff, instance
//! JSONs, harness binaries, the editapply candidate) must be unreachable.
use std::path::Path;
use std::process::Command;

/// Run a probe command inside the real sandbox and return its stdout.
fn probe(scratch: &Path, cmd: &str) -> String {
    let argv = hs_loop::repexec::sandbox_argv(scratch, Path::new("/tmp/o"), Path::new("/tmp/e"), cmd);
    let out = Command::new(&argv[0]).args(&argv[1..]).output().expect("sandbox spawn");
    let so = String::from_utf8_lossy(&out.stdout);
    // sandbox_argv redirects to /ws/.repexec-out; read it back from the scratch
    let captured = std::fs::read_to_string(scratch.join(".repexec-out")).unwrap_or_default();
    format!("{so}{captured}")
}

fn mkscratch() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("f.rs"), "fn main() {}\n").unwrap();
    d
}

#[test]
fn run_dir_and_instance_material_unreachable_inside_sandbox() {
    let d = mkscratch();
    // Create decoy "gold" material at the kinds of host paths missions use.
    let fake_run = tempfile::tempdir().unwrap();
    std::fs::write(fake_run.path().join("test_patch.diff"), "diff --git GOLD ANSWER\n").unwrap();
    let out = probe(d.path(), "ls /mnt /home 2>&1; echo ---; find / -name 'test_patch.diff' -o -name '*.bench-*' 2>/dev/null | head");
    assert!(!out.contains("GOLD ANSWER"), "gold material leaked into sandbox: {out}");
    assert!(!out.contains("test_patch.diff"), "test_patch.diff reachable inside sandbox: {out}");
    for banned in ["/mnt/instinct-nvme", "/home/sandbox", ".hs-eval.patch"] {
        assert!(!out.contains(banned), "harness path visible inside sandbox: {banned} in {out}");
    }
}

#[test]
fn no_hs_env_leaks_into_sandbox() {
    let d = mkscratch();
    std::env::set_var("HS_SWE_F2P", "bash /secret/f2p.sh");
    std::env::set_var("HS_DEEPSEEK_API_KEY_FILE", "/home/sandbox/.keys/deepseek.key");
    let out = probe(d.path(), "env");
    std::env::remove_var("HS_SWE_F2P");
    std::env::remove_var("HS_DEEPSEEK_API_KEY_FILE");
    assert!(!out.contains("HS_"), "harness env leaked into sandbox: {out}");
    assert!(!out.contains("deepseek.key"), "key path leaked: {out}");
}

#[test]
fn prompt_states_sandbox_ws_path() {
    // smoke-8525 trace: the model burned steps poking at the host run_dir
    // path named in the prompt (/mnt/... invisible in-sandbox). The prompt
    // must state where the repo lives INSIDE the exec sandbox.
    let args = hs_loop::sweprompt::PromptArgs {
        ws: "/mnt/instinct-nvme/swbench/x/runs/y/worktree".into(),
        problem_statement: "p".into(),
        fail_to_pass: vec!["t".into()],
        repo_layout: "src/main.rs\n".into(),
        nudge: String::new(),
        answer_path: "/tmp/a".into(),
        orientation: String::new(),
        mcp_tools: String::new(),
    };
    let p = hs_loop::sweprompt::build_mission_prompt(None, &args);
    assert!(!p.contains("/ws "), "test must not self-satisfy via the ws path");
    assert!(p.contains("repo.exec sees the repo at /ws"),
        "prompt must name the in-sandbox repo path: {}", &p[..p.len().min(400)]);
}
