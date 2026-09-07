//! octodns-1298 live-trajectory findings (2026-09-07): three prompt/surface
//! misdirections that burned ~35 steps of a real mission while the model's
//! fix was already correct.
//!
//! 1. repo.exec's description said "pristine copy" - the model read that as
//!    repo-only and tried to persist venvs, pip installs, and /tmp state
//!    across calls. It must state plainly: EVERY call is a fresh
//!    whole-filesystem sandbox; nothing persists but the repo + candidate.
//! 2. The ATTEMPT ARTIFACT line shows the answer FILE (last submit), not the
//!    current candidate - the model read it as live candidate state and
//!    looped on phantom "stale versions". It must be labeled honestly.
//! 3. The mission prompt named a HOST path (bash /mnt/.../f2p.sh) the
//!    sandbox hides; the model hunted the filesystem for it. The prompt must
//!    carry the sandbox-resolved command instead.

#[test]
fn repoexec_description_states_fresh_sandbox_semantics() {
    let tools = hs_loop::toolschema::builtin_tools();
    let desc = tools
        .iter()
        .find(|t| t["function"]["name"] == "repo.exec")
        .map(|t| t["function"]["description"].as_str().unwrap().to_string())
        .expect("repo.exec present");
    assert!(
        desc.contains("FRESH whole-filesystem sandbox"),
        "must state the whole-filesystem fresh-sandbox semantics: {desc}"
    );
    assert!(
        desc.contains("nothing persists"),
        "must state non-persistence plainly: {desc}"
    );
    assert!(desc.contains("python3"), "must name the interpreter: {desc}");
}

#[test]
fn artifact_section_is_labeled_as_the_graded_answer_file() {
    let s = hs_loop::artifact_section(std::path::Path::new("/x/answer.txt"), "diff --git a/f b/f\n");
    assert!(
        s.contains("answer.submit"),
        "label must name the only writer so the model stops reading it as live candidate state: {s}"
    );
    assert!(
        s.contains("diff --git a/f b/f"),
        "content must be shown: {s}"
    );
    let empty = hs_loop::artifact_section(std::path::Path::new("/x/answer.txt"), "");
    assert!(empty.contains("<none>"), "empty artifact marker kept: {empty}");
}

#[test]
fn mission_prompt_carries_the_sandbox_f2p_command_not_a_host_path() {
    let prompt = hs_loop::sweprompt::build_mission_prompt(
        None,
        &hs_loop::sweprompt::PromptArgs {
            ws: "/ws".into(),
            problem_statement: "bug".into(),
            fail_to_pass: vec!["python3 -m pytest tests/test_x.py::TestX::test_y -x -q".into()],
            repo_layout: "src/".into(),
            nudge: String::new(),
            answer_path: "/a".into(),
            orientation: "python3".into(),
            mcp_tools: String::new(),
        },
    );
    assert!(
        prompt.contains("python3 -m pytest tests/test_x.py::TestX::test_y -x -q"),
        "the sandbox-resolved command must be in the prompt: {prompt}"
    );
    assert!(
        prompt.contains("inside repo.exec"),
        "the prompt must say where to run it: {prompt}"
    );
}
