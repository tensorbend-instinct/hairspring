//! The adversarial verifier seam: prompt construction, verdict parsing,
//! the verdict cache key, and the async round worker (gate-8 waste-only
//! redesign, Eric 2026-09-06).
//!
//! WASTE-ONLY PROOF OBLIGATIONS:
//! - every audit that the synchronous path runs still runs - same rounds,
//!   same effort, same veto-only authority, same cap, malfunction never
//!   blocks;
//! - the async seam changes ONLY who waits: the verdict's inputs (mission,
//!   answer, ledger summary, prior gaps) are frozen at submit, so the
//!   verdict is independent of whether the agent idles or works during
//!   the call;
//! - the verdict cache replays a recorded verdict ONLY when the artifact
//!   (ws snapshot id), the answer text, the ledger evidence key, and the
//!   prior gaps are ALL identical: every byte the audit consults is the
//!   same, so re-asking is a resample of an already-drawn verdict, not
//!   new scrutiny.

use hs_kernel::{ModelCaller, PluginEntry};

/// Capped rounds; the cap is unchanged from the synchronous path.
pub const VERIFIER_MAX_ROUNDS: u32 = 3;

/// A verdict as recorded on the audit stream - the cache's payload.
#[derive(Clone, Debug)]
pub struct RecordedVerdict {
    pub refuted: bool,
    pub findings: Vec<String>,
    pub blocking: String,
}

/// What the worker thread sends back.
pub enum VerdictMsg {
    Decided {
        round: u32,
        v: RecordedVerdict,
        out: hs_kernel::ModelOutcome,
        prompt: String,
        tools: serde_json::Value,
    },
    Malformed {
        round: u32,
        detail: String,
        out: Option<hs_kernel::ModelOutcome>,
        prompt: String,
        tools: serde_json::Value,
    },
    CallFailed {
        round: u32,
        detail: String,
        prompt: String,
        tools: serde_json::Value,
    },
}

/// An in-flight audit round.
pub struct PendingVerdict {
    pub round: u32,
    pub rx: std::sync::mpsc::Receiver<VerdictMsg>,
    pub snapshot_id: Option<String>,
    pub key: String,
    pub fired: std::time::Instant,
}

/// Cache key over EVERY byte the audit consults: the mission, the answer
/// text, the ledger evidence (via the ledger's evidence key), the prior
/// gaps, and the artifact itself (the ws snapshot id - content-addressed,
/// so identical trees hash identically). Sequence numbers and submission
/// receipts are excluded by construction (the ledger evidence key skips
/// answer-path writes); those are bookkeeping, not evidence.
pub fn verdict_key(
    mission: &str,
    answer: &str,
    evidence_key: &str,
    prior_gaps: &[String],
    snapshot_id: &str,
) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    for part in [mission, answer, evidence_key, &prior_gaps.join("\u{1f}"), snapshot_id] {
        h.update(part.as_bytes());
        h.update(b"\x1e");
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Item 3: the verifier's prompt. Audit-recorded-evidence only;
/// default-refuted on uncertainty; anti-ratchet on re-rounds
/// (docs/verifier-design.md; Grok goal_verifier_prompt.md adapted).
pub fn build_verifier_prompt(
    mission: &str,
    answer: &str,
    ledger: &crate::ledger::Ledger,
    prior_gaps: &[String],
) -> String {
    let gaps = if prior_gaps.is_empty() {
        "none".to_string()
    } else {
        prior_gaps
            .iter()
            .map(|g| format!("- {g}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut p = String::new();
    p.push_str("ADVERSARIAL VERIFIER\n");
    p.push_str("You are not the agent that did this work. Default to refuted when uncertain a required criterion holds; never invent requirements. Audit the RECORDED evidence only - a prose claim of test output with no recorded run is fabricated: refute. On a re-verification round (PRIOR_GAPS non-empty), check that each prior gap is genuinely fixed plus demonstrable defects; a fresh stylistic objection a prior round implicitly accepted is out of scope - when every prior gap is fixed and the objective holds, return refuted false.\n");
    p.push_str(&format!("OBJECTIVE: {mission}\n"));
    p.push_str(&format!("ANSWER:\n{answer}\n"));
    p.push_str(&format!("LEDGER (recorded evidence):\n{}\n", ledger.summary()));
    p.push_str(&format!("PRIOR_GAPS:\n{gaps}\n"));
    p.push_str("Submit the verdict by calling the verdict.submit tool exactly once - never prose, never bare JSON.");

    p
}

/// Native verdict parsing (one rule set for the sync and async paths):
/// the completion must be a verdict.submit tool call; prose or wrong-tool
/// replies are verifier errors, never parsed verdicts. Error strings are
/// load-bearing - the audit stream's verifier_error details match them.
pub fn parse_verdict(completion: &str) -> Result<RecordedVerdict, String> {
    let verdict_args = serde_json::from_str::<serde_json::Value>(completion.trim())
        .ok()
        .filter(|env| env["tool"].as_str() == Some("verdict.submit"))
        .map(|env| env["args"].clone());
    match verdict_args {
        Some(v) => match v["refuted"].as_bool() {
            Some(b) => Ok(RecordedVerdict {
                refuted: b,
                findings: v["findings"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|f| f["detail"].as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                blocking: v["blocking"].as_str().unwrap_or("none").to_string(),
            }),
            None => Err("verdict JSON missing the refuted field".to_string()),
        },
        None => Err("verdict was not a verdict.submit tool call (prose/wrong-tool reply)".to_string()),
    }
}

/// Fire one audit round on a worker thread with its OWN model-plugin
/// process, built from the same config entry the kernel uses - command,
/// lease, and supervisor semantics identical to a synchronous call.
pub fn spawn_round(
    entry: PluginEntry,
    prompt: String,
    tools: serde_json::Value,
    round: u32,
) -> std::sync::mpsc::Receiver<VerdictMsg> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut caller = ModelCaller::from_entry(entry);
        let msg = match caller.call(&prompt, Some(&tools)) {
            Ok(out) => match parse_verdict(&out.completion) {
                Ok(v) => VerdictMsg::Decided { round, v, out, prompt, tools },
                Err(detail) => VerdictMsg::Malformed { round, detail, out: Some(out), prompt, tools },
            },
            Err(e) => VerdictMsg::CallFailed { round, detail: format!("verifier call failed: {e}"), prompt, tools },
        };
        let _ = tx.send(msg);
    });
    rx
}
