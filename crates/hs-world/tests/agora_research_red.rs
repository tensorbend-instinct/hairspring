use hs_world::research::*;
use uuid::Uuid;
fn c(author: &str, kind: ContributionKind, parents: Vec<Uuid>) -> Contribution {
    Contribution {
        id: Uuid::new_v4(),
        author: author.into(),
        kind,
        description: "evidence".into(),
        parents,
        metric: None,
        artifact_hashes: vec![],
        verification_target: None,
        verdict: None,
        tags: vec![],
    }
}
#[test]
fn cross_author_evidence_negative_results_and_exploration_survive_replay() {
    let d = tempfile::tempdir().unwrap();
    let g = ResearchGraph::open(d.path()).unwrap();
    let root = c("alice", ContributionKind::Result, vec![]);
    g.publish(&root).unwrap();
    let self_child = c("alice", ContributionKind::Result, vec![root.id]);
    g.publish(&self_child).unwrap();
    let other = c("bob", ContributionKind::Result, vec![root.id]);
    g.publish(&other).unwrap();
    let hyp = c("carol", ContributionKind::Hypothesis, vec![]);
    g.publish(&hyp).unwrap();
    let mut fail = c("dave", ContributionKind::Verification, vec![root.id]);
    fail.verification_target = Some(root.id);
    fail.verdict = Some(VerificationVerdict::Failed);
    g.publish(&fail).unwrap();
    let a = ResearchGraph::open(d.path()).unwrap().analyze().unwrap();
    let r = a
        .leaders
        .iter()
        .find(|x| x.contribution.id == root.id)
        .unwrap();
    assert_eq!(
        r.evidence_score, -15,
        "bob's result +5 and dave's failed reproduction -20; alice self-citation excluded"
    );
    assert!(
        a.open_hypotheses
            .iter()
            .any(|x| x.contribution.id == hyp.id)
    );
    assert!(
        a.neglected_leaves
            .iter()
            .any(|x| x.contribution.id == hyp.id)
    );
}
#[test]
fn newest_verdict_replaces_old_and_self_verification_is_rejected() {
    let d = tempfile::tempdir().unwrap();
    let g = ResearchGraph::open(d.path()).unwrap();
    let root = c("alice", ContributionKind::Result, vec![]);
    g.publish(&root).unwrap();
    let mut v = c("bob", ContributionKind::Verification, vec![root.id]);
    v.verification_target = Some(root.id);
    v.verdict = Some(VerificationVerdict::Failed);
    g.publish(&v).unwrap();
    let mut v2 = c("bob", ContributionKind::Verification, vec![root.id]);
    v2.verification_target = Some(root.id);
    v2.verdict = Some(VerificationVerdict::Confirmed);
    g.publish(&v2).unwrap();
    assert_eq!(g.analyze().unwrap().leaders[0].evidence_score, 20);
    let mut own = c("alice", ContributionKind::Verification, vec![root.id]);
    own.verification_target = Some(root.id);
    own.verdict = Some(VerificationVerdict::Confirmed);
    assert!(g.publish(&own).is_err());
}
