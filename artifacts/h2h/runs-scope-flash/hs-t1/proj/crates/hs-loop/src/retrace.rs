//! RETRACE-style patch verification (arXiv 2608.08950), exploratory.
//! After the agent submits, rebuild the problem from the PATCH ALONE
//! (backward), compare it with the real issue (reconcile), and send targeted
//! revision guidance back only on a mismatch. Pure: the model call is a
//! closure so the logic is testable without a model.

#[must_use]
pub fn backward_prompt(diff: &str) -> String {
    format!(
        "Below is a code patch. You have NOT seen the bug report. From the patch alone, write the bug or feature request it most plausibly fixes: the symptom, the trigger condition, and the expected behavior. Be specific; 5 sentences at most.\n\nPATCH:\n{diff}"
    )
}

#[must_use]
pub fn reconcile_prompt(issue: &str, inferred: &str, diff: &str) -> String {
    format!(
        "ISSUE (the real task):\n{issue}\n\nPROBLEM INFERRED FROM THE PATCH ALONE:\n{inferred}\n\nPATCH:\n{diff}\n\nDoes the patch address the real issue completely? Look for: a different trigger case than the issue describes, missing edge cases the issue mentions, behaviour changed beyond the issue. Reply with exactly one line starting `MATCH` if it fully matches, or `MISMATCH:` followed by concrete revision guidance (what to change and where) if not."
    )
}

/// Returns revision feedback when the reconcile verdict is a mismatch.
pub fn review(
    issue: &str,
    diff: &str,
    mut ask: impl FnMut(&str) -> Option<String>,
) -> Option<String> {
    let inferred = ask(&backward_prompt(diff))?;
    let verdict = ask(&reconcile_prompt(issue, inferred.trim(), diff))?;
    let v = verdict.trim();
    let line = v.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    if line.trim_start().to_uppercase().starts_with("MISMATCH") {
        let guidance = v.trim_start().trim_start_matches(|c: char| c.is_alphabetic()).trim_start_matches(':').trim();
        Some(format!(
            "answer.submit HELD for one independent review: a reader who saw only your patch inferred a problem that may differ from the issue. Reviewer guidance: {guidance}\nIf you agree, fix it, re-run the checks, and submit again; if you disagree, submit again unchanged (this review happens once)."
        ))
    } else {
        None
    }
}
