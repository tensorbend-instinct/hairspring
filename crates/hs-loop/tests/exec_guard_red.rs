//! D7 RED (Eric 2026-09-09 postmortem, live `DeepSeek` run 1): the
//! exec-before-patch guard answered "no patch to test yet - build your
//! fix first (edit.patch), then exec" - it never said WHAT repo.exec is
//! for, so the model looped on it. THE LAW after D7: the guard states
//! the precondition plainly - repo.exec runs build/test against the
//! candidate patch written by edit.patch; with no patch on disk there
//! is nothing to test.

#[test]
fn exec_without_patch_states_the_precondition_plainly() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    let missing = dir.path().join("no-answer.txt");
    let v = hs_loop::repexec::run_sandboxed(&ws, &missing, "echo hi", 10);
    assert_eq!(v["applied"].as_bool(), Some(false));
    let note = v["note"].as_str().unwrap_or("");
    assert!(
        note.contains("repo.exec runs build/test against the candidate patch"),
        "the guard must say what repo.exec is for, got: {note}"
    );
    assert!(
        note.contains("edit.patch"),
        "the guard must name the patch tool, got: {note}"
    );
}
