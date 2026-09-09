//! GATE 9e (spec v5; Zhao & Zhao arXiv:2609.00546 - v1 preprint, imported as
//! design rules): capability-vs-fitness attribution.
//!
//! A model or harness swap can lift every score overnight with zero evolved
//! improvement. When a `capability_change` event sits between two assay
//! results, the delta is attributed to the swap, recorded against the NEW
//! binding, and EXCLUDED from the fitness slope. The improvement-cadence
//! protocol reads fitness deltas only: same substrate, same bindings,
//! evolved policy.
//!
//! Projected from the canonical log (Score + `CapabilityChange` events) - a
//! read path, never a parallel store. Single-writer discipline: whoever
//! performs the swap records it on the scorer's stream via
//! `Scorer::record_capability_change` before the next assay.

use hs_core::{Event, EventKind};
use std::collections::HashMap;
use uuid::Uuid;

/// A score delta between consecutive assays of one candidate with NO
/// `capability_change` between them: evolved improvement (or regression),
/// same substrate, same bindings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FitnessDeltaRec {
    pub candidate: String,
    pub from_correct: u32,
    pub to_correct: u32,
    pub total: u32,
    pub delta: i64,
}

/// A score delta straddling a `capability_change` boundary: attributed to the
/// swap, recorded against the new binding, excluded from the fitness slope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttributedDelta {
    pub candidate: String,
    pub from_correct: u32,
    pub to_correct: u32,
    pub total: u32,
    pub delta: i64,
    pub new_binding: String,
    pub boundary_event: Uuid,
}

fn parse_score(body: &str) -> Option<(String, u32, u32)> {
    let mut candidate = None;
    let mut counts = None;
    for tok in body.split_whitespace() {
        if let Some(c) = tok.strip_prefix("candidate=") {
            candidate = Some(c.to_string());
        } else if tok.contains('/') && !tok.contains('=') {
            if let Some((a, b)) = tok.split_once('/') {
                if let (Ok(x), Ok(y)) = (a.parse::<u32>(), b.parse::<u32>()) {
                    counts = Some((x, y));
                }
            }
        }
    }
    match (candidate, counts) {
        (Some(c), Some((x, y))) => Some((c, x, y)),
        _ => None,
    }
}

fn parse_binding(body: &str) -> Option<String> {
    body.split_whitespace()
        .find_map(|t| t.strip_prefix("binding=").map(std::string::ToString::to_string))
}

/// Walk the stream in order. Returns (fitness deltas, capability-attributed
/// deltas). Boundary semantics are positional: ANY `capability_change` event
/// between two consecutive assays of the same candidate excludes that pair
/// from fitness, regardless of whether the binding name changed.
pub fn project(
    events: &[Event],
    resolve: &dyn Fn(&Event) -> Option<String>,
) -> (Vec<FitnessDeltaRec>, Vec<AttributedDelta>) {
    let mut fitness = vec![];
    let mut attributed = vec![];
    // candidate -> (correct, total, seq of that assay)
    let mut last_score: HashMap<String, (u32, u32, u64)> = HashMap::new();
    // (seq, event_id, binding) of the most recent capability_change
    let mut last_cc: Option<(u64, Uuid, String)> = None;

    for e in events {
        match e.kind {
            EventKind::CapabilityChange => {
                let binding = resolve(e)
                    .and_then(|b| parse_binding(&b))
                    .unwrap_or_default();
                last_cc = Some((e.seq, e.event_id, binding));
            }
            EventKind::Score => {
                let Some(body) = resolve(e) else { continue };
                let Some((cand, correct, total)) = parse_score(&body) else {
                    continue;
                };
                if let Some((pc, ptotal, pseq)) = last_score.get(&cand).copied() {
                    let delta = i64::from(correct) - i64::from(pc);
                    let boundary = last_cc.as_ref().filter(|(s, _, _)| *s > pseq);
                    match boundary {
                        None => fitness.push(FitnessDeltaRec {
                            candidate: cand.clone(),
                            from_correct: pc,
                            to_correct: correct,
                            total: ptotal.max(total),
                            delta,
                        }),
                        Some((_, cc_id, binding)) => attributed.push(AttributedDelta {
                            candidate: cand.clone(),
                            from_correct: pc,
                            to_correct: correct,
                            total: ptotal.max(total),
                            delta,
                            new_binding: binding.clone(),
                            boundary_event: *cc_id,
                        }),
                    }
                }
                last_score.insert(cand, (correct, total, e.seq));
            }
            _ => {}
        }
    }
    (fitness, attributed)
}
