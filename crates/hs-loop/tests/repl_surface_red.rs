//! Dance #94 RED (Eric 2026-09-09, live DeepSeek-v4-pro failure record):
//! the REPL advertised the hardcoded Terminal-Bench tool surface
//! (tb_tools: term.exec, ...) regardless of which tools the run config
//! actually registered. Live cost: 58 model calls, $8.81 ledger spend, zero
//! files written - the model called the advertised term.exec, the kernel
//! answered "unknown tool" (never registered), and repo.exec/edit.patch
//! were dispatchable but never advertised.
//!
//! THE LAW after D1: the advertised surface is DERIVED from the kernel
//! registry plus the loop's internally-dispatched tools - advertised ==
//! dispatchable, by construction. No second hand-written menu.
//!
//! D2: dispatch normalizes the observed demangled mistake shapes
//! (term.exec where term__exec was served, term_exec, term) against the
//! registered names. D3: an unknown-tool error must TEACH - it names every
//! tool the caller may use.

use hs_loop::repl::ReplSession;
use hs_loop::InnerLoop;

const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");
const ANSWERSUBMIT: &str = env!("CARGO_BIN_EXE_hs-plugin-answersubmit");
const SELFCHECK: &str = env!("CARGO_BIN_EXE_hs-plugin-selfcheck");
const FILEREAD: &str = env!("CARGO_BIN_EXE_hs-plugin-fileread");
const REPOSEARCH: &str = env!("CARGO_BIN_EXE_hs-plugin-reposearch");
const REPOEXEC: &str = env!("CARGO_BIN_EXE_hs-plugin-repoexec");
const APPLYPATCH: &str = env!("CARGO_BIN_EXE_hs-plugin-applypatch");
const NOTESCRATCH: &str = env!("CARGO_BIN_EXE_hs-plugin-notescratch");
const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");

static SEQMODEL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn write(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p
}

/// The SWE-shaped config the real run used (realrun1/hairspring.toml).
fn swe_config(dir: &std::path::Path, model_bin: &str) -> std::path::PathBuf {
    write(
        dir,
        "hairspring.toml",
        &format!(
            r#"
[[tools]]
name = "answer.submit"
command = ["{ANSWERSUBMIT}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{SELFCHECK}"]
subjects = ["*"]

[[tools]]
name = "repo.read"
command = ["{FILEREAD}"]
subjects = ["*"]

[[tools]]
name = "repo.search"
command = ["{REPOSEARCH}"]
subjects = ["*"]

[[tools]]
name = "repo.exec"
command = ["{REPOEXEC}"]
subjects = ["*"]

[[tools]]
name = "edit.patch"
command = ["{APPLYPATCH}"]
subjects = ["*"]

[[tools]]
name = "notes.scratch"
command = ["{NOTESCRATCH}"]
subjects = ["*"]

[[models]]
name = "scripted"
command = ["{model_bin}"]
default = true
"#
        ),
    )
}

/// The tools a REPL session dispatches internally (never in the TOML, never
/// registered in the kernel): swarm delegation, the world plane, and the
/// memory plane (present when the session db opens, which a writable log
/// root guarantees).
const INTERNALS: [&str; 7] = [
    "agent.spawn",
    "agent.spawn_poll",
    "world.propose",
    "world.observe",
    "world.install",
    "world.tick",
    "memory.recall",
];

#[test]
fn repl_advertises_exactly_the_registered_tools_plus_internals() {
    let _g = SEQMODEL_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let script = write(dir.path(), "script.jsonl", "\"probe done\"\n");
    unsafe {
        std::env::remove_var("HS_MCP_SERVERS");
        std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
    }
    let session = ReplSession::load(&swe_config(dir.path(), SCRIPTED), log.path(), false, 4)
        .expect("session load over a SWE-shaped config");
    unsafe {
        std::env::remove_var("HS_SEQMODEL_SCRIPT");
    }
    let advertised: std::collections::BTreeSet<String> =
        session.native_tool_names().into_iter().collect();

    let config_tools = [
        "answer.submit",
        "checker.run",
        "repo.read",
        "repo.search",
        "repo.exec",
        "edit.patch",
        "notes.scratch",
    ];
    let mut want: std::collections::BTreeSet<String> =
        config_tools.iter().map(|s| (*s).to_string()).collect();
    want.extend(INTERNALS.iter().map(|s| (*s).to_string()));

    assert_eq!(
        advertised, want,
        "advertised surface must equal registered tools + loop internals \
         (live failure 2026-09-09: term.exec advertised but unregistered, \
         repo.exec/edit.patch registered but unadvertised)"
    );
    assert!(
        !advertised.contains("term.exec"),
        "TB flavor must not leak into a SWE-config session: {advertised:?}"
    );
}

#[test]
fn dispatch_normalizes_demangled_names_and_unknown_errors_teach() {
    let _g = SEQMODEL_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let answer = log.path().join("work").join("task-20").join("answer.txt");
    // Pre-seat an answer so the end-of-mission checker always has a task;
    // the assertions here are about dispatch/error SHAPE, not the verdict.
    std::fs::create_dir_all(answer.parent().unwrap()).unwrap();
    std::fs::write(&answer, "TOKEN-20-SECRET").unwrap();
    // Step 1: the observed DeepSeek mistake shape - demangled single
    // underscore for a registered dotted name. Must DISPATCH (D2).
    // Step 2: a truly unknown tool. The error fed back must NAME the valid
    // tools (D3) - "unknown tool: X" alone looped the live model 8 times.
    // Step 3: stand down so the checker can run.
    let script = write(
        dir.path(),
        "script.jsonl",
        &format!(
            "{{\"tool\":\"answer_write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-20-SECRET\"}}}}\n{{\"tool\":\"bogus.noop\",\"args\":{{}}}}\n\"done, answer written\"\n",
            answer.display()
        ),
    );
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let config = write(
        dir.path(),
        "hairspring.toml",
        &format!(
            r#"
[[tools]]
name = "answer.write"
command = ["{ANSWER}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{CHECKER}"]
subjects = ["*"]

[[models]]
name = "scripted"
command = ["{SCRIPTED}"]
default = true
"#
        ),
    );
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 4).unwrap();
    let r = l.run_mission("task-20").unwrap();
    unsafe { std::env::remove_var("HS_SEQMODEL_SCRIPT") };

    let reader = hs_log::StreamReader::open(log.path(), r.stream_id).unwrap();
    let events: Vec<_> = reader.events().unwrap();

    // D2: the demangled answer_write call dispatched to answer.write.
    let dispatched_write = events.iter().any(|e| {
        e.kind == hs_core::EventKind::ToolCall && {
            let b = reader.resolve_payload(e).unwrap();
            let s = String::from_utf8_lossy(&b);
            s.contains("\"plugin\": \"answer.write\"") || s.contains("\"plugin\":\"answer.write\"")
        }
    });
    assert!(
        dispatched_write,
        "demangled variant answer_write must resolve to registered answer.write"
    );

    // D3: the bogus.noop error the model is SHOWN names the valid tools.
    let prompts: Vec<String> = events
        .iter()
        .filter(|e| e.kind == hs_core::EventKind::ModelCall)
        .map(|e| {
            let b = reader.resolve_payload(e).unwrap();
            let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
            hs_loop::msgfmt::prompt_view(&v)
        })
        .collect();
    let later = prompts.last().cloned().unwrap_or_default();
    assert!(
        later.contains("answer.write") && later.contains("checker.run"),
        "unknown-tool feedback must list the valid tool names: {}",
        &later[..later.len().min(600)]
    );
}
