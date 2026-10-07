use hs_loop::retrace::{backward_prompt, review};

#[test]
fn backward_pass_never_sees_the_issue_and_mismatch_returns_guidance() {
    let issue = "ISSUE-TEXT-UNIQUE: crash when list is empty";
    let diff = "--- a/x.py\n+++ b/x.py\n@@ -1 +1 @@\n-a\n+b\n";
    let mut prompts: Vec<String> = vec![];
    let r = review(issue, diff, |q| {
        prompts.push(q.to_string());
        Some(if prompts.len() == 1 {
            "Fixes a typo.".to_string()
        } else {
            "MISMATCH: also handle the empty list in x.py".to_string()
        })
    });
    assert!(!prompts[0].contains("ISSUE-TEXT-UNIQUE"), "backward pass leaked the issue");
    assert!(prompts[0].contains("+b") && prompts[1].contains("ISSUE-TEXT-UNIQUE") && prompts[1].contains("Fixes a typo."));
    let msg = r.expect("mismatch yields feedback");
    assert!(msg.contains("also handle the empty list") && msg.contains("once"), "{msg}");
    assert!(backward_prompt("d").contains("NOT seen"));
}

#[test]
fn match_verdict_or_model_failure_lets_the_submission_through() {
    let m = review("i", "d", |_| Some("MATCH".to_string()));
    assert!(m.is_none());
    assert!(review("i", "d", |_| None).is_none());
}
