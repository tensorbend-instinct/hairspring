//! RED: the shipped live rig registers agent.spawn + agent.spawn_poll in
//! its own [[tools]] (config comment: "Async sub-agent delegation"), and
//! the REPL's interactive surface pushed its OWN delegation pair on top
//! of the registry-derived list - the operator's advertised tools carried
//! both copies, the wire translation (dots -> __) produced duplicate
//! function names, and DeepSeek rejected EVERY live mission with
//! "Tool names must be unique" (live 400, replayed against the provider
//! 2026-09-10). The offered surface must name each tool exactly once.

use hs_loop::repl::ReplSession;

#[test]
fn offered_tool_names_are_unique_when_config_registers_delegation() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    // Mirror the shipped rig's delegation pair: the config registers the
    // swarm binary as BOTH agent.spawn and agent.spawn_poll.
    let config = dir.path().join("hairspring.toml");
    // the swarm plugin lives in the hs-swarm crate (outside this package's
    // CARGO_BIN_EXE reach): resolve it from the workspace target dir.
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    let target = std::env::var("CARGO_TARGET_DIR")
        .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../target").to_string());
    let swarm = format!("{target}/{profile}/hs-plugin-swarm");
    std::fs::write(
        &config,
        format!(
            r#"
[[tools]]
name = "answer.submit"
command = ["{}"]
subjects = ["*"]

[[tools]]
name = "agent.spawn"
command = ["{}"]
subjects = ["*"]

[[tools]]
name = "agent.spawn_poll"
command = ["{}", "--as", "agent.spawn_poll"]
subjects = ["*"]

[[models]]
name = "recmodel"
command = ["{}"]
default = true
subjects = ["*"]
"#,
            env!("CARGO_BIN_EXE_hs-plugin-answersubmit"),
            swarm,
            swarm,
            env!("CARGO_BIN_EXE_hs-plugin-recmodel")
        ),
    )
    .unwrap();
    let dump = dir.path().join("calls.txt");
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("REC_DUMP", &dump) };
    let mut s = ReplSession::load(&config, log.path(), false, 1).expect("session load");
    // one step is enough: the first model.call carries the offered surface
    let _ = s.run_goal("offered surface probe");
    let text = std::fs::read_to_string(&dump).expect("recmodel dumped its call");
    let tools_block = text
        .split("===TOOLS===\n")
        .nth(1)
        .expect("dump carries the offered tools");
    let names: Vec<&str> = tools_block
        .lines()
        .next()
        .unwrap_or("")
        .split(',')
        .filter(|n| !n.is_empty())
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    let dupes: Vec<&&str> = names.iter().filter(|n| !seen.insert(*n)).collect();
    assert!(
        dupes.is_empty(),
        "offered surface carries duplicate tools (provider 400s): {dupes:?} in {names:?}"
    );
    assert!(
        names.contains(&"agent.spawn"),
        "the delegation tool is still offered once: {names:?}"
    );
}
