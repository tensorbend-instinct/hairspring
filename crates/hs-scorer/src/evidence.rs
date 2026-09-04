//! GATE 9c (spec v5; Yan et al., arXiv:2609.01481 - v1 preprint, imported as
//! design rules): evidence state as a first-class record.
//!
//! An artifact says what exists; evidence says what is KNOWN about it. The
//! record is projected from the canonical log (Score / Regression events) -
//! a read path, never a parallel store. Regression bookkeeping is tracked,
//! not overwritten: a claim verified at event N that fails at event M
//! becomes a regression record pointing at both, so "it worked before" is a
//! query over the record.
//!
//! Ownership: the scorer observes every assay, so the scorer emits the
//! Regression event (single-writer discipline on its stream).

use hs_core::{Event, EventKind};
use sha2::Digest;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimKind {
    VerifiedClaim,
    OpenFailure,
    Regression,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimStatus {
    Open,
    Closed,
    Superseded,
}

/// One claim about a subject (artifact / candidate / capability ref).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceClaim {
    pub claim_id: Uuid,
    pub subject: String,
    pub kind: ClaimKind,
    pub verified_at: Option<Uuid>,
    pub regressed_at: Option<Uuid>,
    pub evidence_ref: String,
    pub status: ClaimStatus,
}

impl EvidenceClaim {
    fn for_subject(subject: &str) -> Self {
        // stable claim id: sha256(subject)[..16] - the same claim surfaces
        // with the same id on every projection
        let digest = sha2::Sha256::digest(subject.as_bytes());
        let mut id = [0u8; 16];
        id.copy_from_slice(&digest[..16]);
        Self {
            claim_id: Uuid::from_bytes(id),
            subject: subject.into(),
            kind: ClaimKind::OpenFailure,
            verified_at: None,
            regressed_at: None,
            evidence_ref: String::new(),
            status: ClaimStatus::Open,
        }
    }
}

/// Parse a Score event body: "tier01 candidate=<s> suite=<s> passed=<b> n/n".
/// Returns (subject, passed).
pub fn parse_score(body: &str) -> Option<(String, bool)> {
    let mut subject = None;
    let mut passed = None;
    for tok in body.split_whitespace() {
        if let Some(v) = tok.strip_prefix("candidate=") {
            subject = Some(v.to_string());
        }
        if let Some(v) = tok.strip_prefix("passed=") {
            passed = Some(v == "true");
        }
    }
    Some((subject?, passed?))
}

/// Parse a Regression event body:
/// "regression subject=<s> verified_at=<uuid> regressed_at=<uuid>".
pub fn parse_regression(body: &str) -> Option<(String, Uuid, Uuid)> {
    let mut subject = None;
    let mut n = None;
    let mut m = None;
    for tok in body.split_whitespace() {
        if let Some(v) = tok.strip_prefix("subject=") {
            subject = Some(v.to_string());
        }
        if let Some(v) = tok.strip_prefix("verified_at=") {
            n = Uuid::parse_str(v).ok();
        }
        if let Some(v) = tok.strip_prefix("regressed_at=") {
            m = Uuid::parse_str(v).ok();
        }
    }
    Some((subject?, n?, m?))
}

/// Project evidence state from the event record. Pure read path.
pub fn project(events: &[Event], resolve: &dyn Fn(&Event) -> Option<String>) -> Vec<EvidenceClaim> {
    let mut claims: Vec<EvidenceClaim> = vec![];
    for e in events {
        let Some(body) = resolve(e) else { continue };
        match e.kind {
            EventKind::Score => {
                if let Some((subject, passed)) = parse_score(&body) {
                    let idx = match claims.iter().position(|c| c.subject == subject) {
                        Some(i) => i,
                        None => {
                            claims.push(EvidenceClaim::for_subject(&subject));
                            claims.len() - 1
                        }
                    };
                    if passed {
                        // re-verification after a regression supersedes it:
                        // the regression record stays in the log either way
                        if claims[idx].kind == ClaimKind::Regression {
                            claims[idx].status = ClaimStatus::Superseded;
                            let mut fresh = EvidenceClaim::for_subject(&subject);
                            fresh.kind = ClaimKind::VerifiedClaim;
                            fresh.verified_at = Some(e.event_id);
                            fresh.evidence_ref = body.clone();
                            claims.push(fresh);
                        } else {
                            claims[idx].kind = ClaimKind::VerifiedClaim;
                            claims[idx].verified_at = Some(e.event_id);
                            claims[idx].evidence_ref = body.clone();
                        }
                    } else if claims[idx].kind != ClaimKind::Regression {
                        claims[idx].kind = ClaimKind::OpenFailure;
                        claims[idx].evidence_ref = body.clone();
                    }
                }
            }
            EventKind::Regression => {
                if let Some((subject, n, m)) = parse_regression(&body) {
                    let idx = match claims.iter().position(|c| c.subject == subject) {
                        Some(i) => i,
                        None => {
                            claims.push(EvidenceClaim::for_subject(&subject));
                            claims.len() - 1
                        }
                    };
                    claims[idx].kind = ClaimKind::Regression;
                    claims[idx].verified_at = Some(n);
                    claims[idx].regressed_at = Some(m);
                    claims[idx].evidence_ref = body.clone();
                }
            }
            _ => {}
        }
    }
    claims
}
