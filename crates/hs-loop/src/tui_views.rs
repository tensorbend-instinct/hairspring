//! B8 TUI read views: project the selfmod chain's REGISTERED substrate
//! streams (B8a role registry) into typed view state for the overlays
//! (checklist 1.3 scorer surface, 1.5 lineage view).
//!
//! Deliberate layering: this module consumes the substrate records via
//! `hs-log` + `hs-core` ONLY - the TUI reads what the log holds and
//! never links the scorer/selfmod components themselves. Payload shapes
//! are the emitters' verbatim formats (hs-scorer `emit` sites for
//! `scorer_pin`/score/canary lines, hs-selfmod `emit` sites for the
//! mutation JSON and the delta lines). Events that do not conform are
//! skipped - corruption detection belongs to the substrate verifier
//! (`hs-log-cli verify`), the view shows what the log verifiably holds.

use hs_core::EventKind;
use std::path::Path;

/// A booked mutation: which fork, how many policy changes.
#[derive(Clone, Debug)]
pub struct MutationRecord {
    /// The fork's stream.
    pub fork: uuid::Uuid,
    /// Policy changes in the mutation.
    pub changes: usize,
}

/// A booked promotion delta.
#[derive(Clone, Debug)]
pub struct CapabilityDelta {
    /// Promoted candidate name.
    pub candidate: String,
    /// Always a promotion when booked.
    pub promoted: bool,
    /// Prompt count after the promotion.
    pub prompts: u64,
    /// Tool count after the promotion.
    pub tools: u64,
}

/// A booked held-out fitness delta.
#[derive(Clone, Debug)]
pub struct FitnessDelta {
    /// Candidate name.
    pub candidate: String,
    /// Held-out pass rate at promotion.
    pub held_out_pass_rate: f64,
}

/// The lineage view over the registered `selfmod` stream.
#[derive(Clone, Debug, Default)]
pub struct SelfmodView {
    /// Mutations, in log order.
    pub mutations: Vec<MutationRecord>,
    /// Capability deltas, in log order.
    pub capability_deltas: Vec<CapabilityDelta>,
    /// Fitness deltas, in log order.
    pub fitness_deltas: Vec<FitnessDelta>,
}

/// A booked scorer pin (version + assay conditions + hash).
#[derive(Clone, Debug)]
pub struct ScorerPinRecord {
    /// Scorer version string.
    pub version: String,
    /// Assay conditions.
    pub conditions: String,
    /// Pin hash (hex).
    pub hash: String,
}

/// A booked score line (any tier).
#[derive(Clone, Debug)]
pub struct ScoreRecord {
    /// Tier label (`tier01`, ...).
    pub tier: String,
    /// Candidate name.
    pub candidate: String,
    /// Suite name.
    pub suite: String,
    /// Pass verdict.
    pub passed: bool,
    /// Tasks correct.
    pub correct: u32,
    /// Tasks total.
    pub total: u32,
}

/// A booked canary result.
#[derive(Clone, Debug)]
pub struct CanaryRecord {
    /// Canary id.
    pub id: String,
    /// Planted ground truth.
    pub ground_truth_good: bool,
    /// What the scorer said.
    pub scorer_said_good: bool,
    /// Scorer disagreed with ground truth.
    pub error: bool,
}

/// The scorer view over the registered `scorer` stream.
#[derive(Clone, Debug, Default)]
pub struct ScorerView {
    /// Scorer pins, in log order.
    pub pins: Vec<ScorerPinRecord>,
    /// Scores, in log order.
    pub scores: Vec<ScoreRecord>,
    /// Canary results, in log order.
    pub canaries: Vec<CanaryRecord>,
}

/// `k=v` pairs from a payload line's whitespace-separated fields.
fn kv(fields: &[&str]) -> std::collections::HashMap<String, String> {
    fields
        .iter()
        .filter_map(|f| f.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn events_payloads(
    log_root: &Path,
    role: &str,
) -> Option<Vec<(EventKind, String)>> {
    let stream = hs_log::registered_stream(log_root, role)?;
    let reader = hs_log::StreamReader::open(log_root, stream).ok()?;
    let events = reader.events().ok()?;
    Some(
        events
            .iter()
            .map(|e| {
                let body = reader
                    .resolve_payload(e)
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_default();
                (e.kind, body)
            })
            .collect(),
    )
}

/// Project the registered `selfmod` stream into the lineage view.
/// None when no selfmod cycle has registered a stream under this root -
/// the honest empty state, never a fabricated panel.
#[must_use]
pub fn selfmod_view(log_root: &Path) -> Option<SelfmodView> {
    let events = events_payloads(log_root, "selfmod")?;
    let mut view = SelfmodView::default();
    for (kind, body) in events {
        let fields: Vec<&str> = body.split_whitespace().collect();
        match kind {
            EventKind::Mutation => {
                // {"fork": "<uuid>", "changes": [...]} (hs-selfmod apply)
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) else {
                    continue;
                };
                let Some(fork) = v["fork"].as_str().and_then(|s| uuid::Uuid::parse_str(s).ok())
                else {
                    continue;
                };
                let changes = v["changes"].as_array().map_or(0, Vec::len);
                view.mutations.push(MutationRecord { fork, changes });
            }
            EventKind::CapabilityDelta => {
                let m = kv(&fields);
                view.capability_deltas.push(CapabilityDelta {
                    candidate: m.get("candidate").cloned().unwrap_or_default(),
                    promoted: m.get("promoted").is_some_and(|v| v == "true"),
                    prompts: m.get("prompts").and_then(|v| v.parse().ok()).unwrap_or(0),
                    tools: m.get("tools").and_then(|v| v.parse().ok()).unwrap_or(0),
                });
            }
            EventKind::FitnessDelta => {
                let m = kv(&fields);
                view.fitness_deltas.push(FitnessDelta {
                    candidate: m.get("candidate").cloned().unwrap_or_default(),
                    held_out_pass_rate: m
                        .get("held_out_pass_rate")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0.0),
                });
            }
            _ => {}
        }
    }
    Some(view)
}

/// Project the registered `scorer` stream into the scorer view. None
/// when no scorer has registered a stream under this root.
#[must_use]
pub fn scorer_view(log_root: &Path) -> Option<ScorerView> {
    let events = events_payloads(log_root, "scorer")?;
    let mut view = ScorerView::default();
    for (kind, body) in events {
        let fields: Vec<&str> = body.split_whitespace().collect();
        match kind {
            EventKind::ScorerPin => {
                let m = kv(&fields);
                view.pins.push(ScorerPinRecord {
                    version: m.get("version").cloned().unwrap_or_default(),
                    conditions: m.get("conditions").cloned().unwrap_or_default(),
                    hash: m.get("hash").cloned().unwrap_or_default(),
                });
            }
            EventKind::Score => {
                // "tier01 candidate=C suite=S passed=bool <correct>/<total>"
                let Some(tier) = fields.first().map(|s| (*s).to_string()) else {
                    continue;
                };
                let m = kv(&fields);
                let (correct, total) = fields
                    .last()
                    .and_then(|t| t.split_once('/'))
                    .and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))
                    .unwrap_or((0, 0));
                view.scores.push(ScoreRecord {
                    tier,
                    candidate: m.get("candidate").cloned().unwrap_or_default(),
                    suite: m.get("suite").cloned().unwrap_or_default(),
                    passed: m.get("passed").is_some_and(|v| v == "true"),
                    correct,
                    total,
                });
            }
            EventKind::CanaryResult => {
                let m = kv(&fields);
                view.canaries.push(CanaryRecord {
                    id: m.get("id").cloned().unwrap_or_default(),
                    ground_truth_good: m.get("ground_truth_good").is_some_and(|v| v == "true"),
                    scorer_said_good: m.get("scorer_said_good").is_some_and(|v| v == "true"),
                    error: m.get("error").is_some_and(|v| v == "true"),
                });
            }
            _ => {}
        }
    }
    Some(view)
}
