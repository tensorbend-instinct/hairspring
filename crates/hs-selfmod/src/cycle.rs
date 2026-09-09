//! GATE 9d (spec v5; Yan et al., arXiv:2609.01481 - v1 preprint, imported as
//! design rules): the evolutionary loop's cycle rules.
//!
//! Per-cycle balance: each cycle closes an outstanding failure from evidence
//! state AND adds one bounded new capability. Repair-only cycles stall into
//! repetitive local fixes; capability-only cycles accumulate unverified
//! surface. The rule is enforced BEFORE the fork exists - an unbalanced
//! cycle never gets a quarantine stream.
//!
//! Independent acceptance on a frozen candidate is enforced at promote time
//! in `SelfModLoop::promote`: the verdict must name the fork's own candidate.

use crate::Mutation;
use hs_scorer::evidence::{ClaimKind, ClaimStatus, EvidenceClaim};
use uuid::Uuid;

/// One evolutionary cycle proposal: the failure it closes (by claim id,
/// from evidence state), the one bounded capability it adds, and the
/// policy-layer mutation carrying both.
pub struct CycleProposal {
    pub closes_failure: Option<Uuid>,
    pub adds_capability: Option<String>,
    pub mutation: Mutation,
}

/// Cap on the capability description: "one bounded new capability" is a
/// naming discipline, not an essay.
const BOUNDED_CAPABILITY_MAX: usize = 280;

/// Validate the balance rule against the CURRENT evidence state.
pub fn check_balance(
    p: &CycleProposal,
    evidence: &[EvidenceClaim],
) -> Result<(), crate::SelfModError> {
    let repair = p.closes_failure.is_some();
    let capability = p
        .adds_capability
        .as_ref()
        .is_some_and(|c| !c.trim().is_empty());
    if repair && !capability {
        return Err(crate::SelfModError::UnbalancedCycle(
            "repair-only cycle: closing a failure must also add one bounded capability".into(),
        ));
    }
    if capability && !repair {
        return Err(crate::SelfModError::UnbalancedCycle(
            "capability-only cycle: adding capability must also close one open failure".into(),
        ));
    }
    if !repair && !capability {
        return Err(crate::SelfModError::UnbalancedCycle(
            "empty cycle: close one open failure AND add one bounded capability".into(),
        ));
    }
    let cap = p
        .adds_capability
        .as_ref()
        .expect("balance check above requires a non-empty capability");
    if cap.len() > BOUNDED_CAPABILITY_MAX {
        return Err(crate::SelfModError::UnbalancedCycle(format!(
            "capability must be bounded (<= {BOUNDED_CAPABILITY_MAX} chars), got {}",
            cap.len()
        )));
    }
    let claim_id = p
        .closes_failure
        .expect("balance check above requires a closed failure");
    let claim = evidence.iter().find(|c| c.claim_id == claim_id);
    match claim {
        Some(c)
            if c.status == ClaimStatus::Open
                && matches!(c.kind, ClaimKind::OpenFailure | ClaimKind::Regression) =>
        {
            Ok(())
        }
        Some(_) => Err(crate::SelfModError::EvidenceMismatch(
            "claim exists but is not an open failure/regression".into(),
        )),
        None => Err(crate::SelfModError::EvidenceMismatch(
            "closes_failure names no claim in evidence state".into(),
        )),
    }
}
