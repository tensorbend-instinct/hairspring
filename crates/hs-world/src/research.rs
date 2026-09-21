//! Agora-derived shared research graph.
//!
//! HAIRSPRING uses its existing append-only, content-addressed event log rather
//! than introducing a second Git repository. The retained Agora mechanisms are
//! explicit builds-on edges, typed positive and negative contributions,
//! cross-author evidence scoring, replaceable verification verdicts, and an
//! analyze view that deliberately exposes both leaders and neglected leaves.

use hs_core::{EventBuilder, EventKind, Payload};
use hs_log::{StreamReader, StreamWriter};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use uuid::Uuid;

pub fn research_stream_id() -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_DNS, b"hairspring.research.agora")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContributionKind {
    Setup,
    Result,
    Insight,
    Hypothesis,
    Report,
    Verification,
    Endorsed,
    Wip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationVerdict {
    Confirmed,
    Partial,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contribution {
    pub id: Uuid,
    pub author: String,
    pub kind: ContributionKind,
    pub description: String,
    #[serde(default)]
    pub parents: Vec<Uuid>,
    #[serde(default)]
    pub metric: Option<f64>,
    #[serde(default)]
    pub artifact_hashes: Vec<String>,
    #[serde(default)]
    pub verification_target: Option<Uuid>,
    #[serde(default)]
    pub verdict: Option<VerificationVerdict>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedContribution {
    pub contribution: Contribution,
    pub evidence_score: i64,
    pub follow_on_count: usize,
    pub ucb: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchAnalysis {
    pub total: usize,
    pub leaders: Vec<RankedContribution>,
    pub neglected_leaves: Vec<RankedContribution>,
    pub open_hypotheses: Vec<RankedContribution>,
    pub unverified_results: Vec<RankedContribution>,
    pub contested_targets: Vec<Uuid>,
}

fn weight(c: &Contribution) -> i64 {
    match c.kind {
        ContributionKind::Verification => match c.verdict {
            Some(VerificationVerdict::Confirmed) => 20,
            Some(VerificationVerdict::Partial) => 10,
            Some(VerificationVerdict::Failed) => -20,
            None => 0,
        },
        ContributionKind::Setup
        | ContributionKind::Result
        | ContributionKind::Insight
        | ContributionKind::Hypothesis
        | ContributionKind::Report => 5,
        ContributionKind::Endorsed | ContributionKind::Wip => 0,
    }
}

pub struct ResearchGraph {
    root: std::path::PathBuf,
    stream: Uuid,
}
impl ResearchGraph {
    pub fn open(root: &Path) -> Result<Self, hs_log::LogError> {
        let stream = research_stream_id();
        if !root.join("streams").join(stream.to_string()).exists() {
            StreamWriter::create(root, stream)?;
        }
        Ok(Self {
            root: root.to_path_buf(),
            stream,
        })
    }

    pub fn publish(&self, c: &Contribution) -> Result<(), String> {
        if c.author.trim().is_empty() || c.description.trim().is_empty() {
            return Err("author and description are required".into());
        }
        let nodes = self
            .contributions()
            .map_err(|e| format!("read graph: {e:?}"))?;
        let ids: HashSet<_> = nodes.iter().map(|n| n.id).collect();
        if c.parents.iter().any(|p| !ids.contains(p)) {
            return Err("every parent must name an existing contribution".into());
        }
        match c.kind {
            ContributionKind::Hypothesis if c.metric.is_some() => {
                return Err("an untested hypothesis cannot claim a metric".into());
            }
            ContributionKind::Verification => {
                let t = c
                    .verification_target
                    .ok_or("verification requires exactly one target")?;
                let target = nodes
                    .iter()
                    .find(|n| n.id == t)
                    .ok_or("verification target does not exist")?;
                if target.author == c.author {
                    return Err("self-verification is not evidence".into());
                }
                if c.verdict.is_none() {
                    return Err("verification requires a verdict".into());
                }
            }
            _ if c.verification_target.is_some() || c.verdict.is_some() => {
                return Err("only verification contributions carry a target or verdict".into());
            }
            _ => {}
        }
        if c.parents.contains(&c.id) || ids.contains(&c.id) {
            return Err("contribution id must be new and cannot parent itself".into());
        }
        let mut w = StreamWriter::resume(&self.root, self.stream)
            .map_err(|e| format!("open writer: {e:?}"))?
            .writer;
        w.append(
            EventBuilder::new(EventKind::Consequence).payload(Payload::Inline(
                serde_json::to_vec(c).map_err(|e| e.to_string())?,
            )),
        )
        .map_err(|e| format!("append: {e:?}"))?;
        Ok(())
    }

    pub fn contributions(&self) -> Result<Vec<Contribution>, hs_log::LogError> {
        let r = StreamReader::open(&self.root, self.stream)?;
        let mut out = Vec::new();
        for e in r.events()? {
            if e.kind == EventKind::Consequence {
                if let Ok(bytes) = r.resolve_payload(&e) {
                    if let Ok(c) = serde_json::from_slice(&bytes) {
                        out.push(c);
                    }
                }
            }
        }
        Ok(out)
    }

    pub fn analyze(&self) -> Result<ResearchAnalysis, hs_log::LogError> {
        let nodes = self.contributions()?;
        let by_id: HashMap<_, _> = nodes.iter().map(|n| (n.id, n)).collect();
        let mut score: HashMap<Uuid, i64> = HashMap::new();
        let mut children: HashMap<Uuid, usize> = HashMap::new();
        let mut latest_verdict: HashMap<(String, Uuid), &Contribution> = HashMap::new();
        for n in &nodes {
            if let Some(t) = n.verification_target {
                latest_verdict.insert((n.author.clone(), t), n);
            }
            for p in &n.parents {
                *children.entry(*p).or_default() += 1;
            }
        }
        for n in &nodes {
            if n.kind == ContributionKind::Verification {
                if let Some(t) = n.verification_target {
                    if latest_verdict
                        .get(&(n.author.clone(), t))
                        .is_some_and(|v| v.id == n.id)
                    {
                        let different = by_id.get(&t).is_some_and(|p| p.author != n.author);
                        if different {
                            *score.entry(t).or_default() += weight(n);
                        }
                    }
                }
            } else {
                for p in &n.parents {
                    if by_id.get(p).is_some_and(|parent| parent.author != n.author) {
                        *score.entry(*p).or_default() += weight(n);
                    }
                }
            }
        }
        let n_total = nodes.len() as f64;
        let mut ranked: Vec<_> = nodes
            .iter()
            .cloned()
            .map(|c| {
                let follow = *children.get(&c.id).unwrap_or(&0);
                let s = *score.get(&c.id).unwrap_or(&0);
                let quality = c.metric.unwrap_or(0.0);
                let ucb =
                    100.0 * quality + 20.0 * ((n_total + 1.0).ln() / (follow as f64 + 1.0)).sqrt();
                RankedContribution {
                    contribution: c,
                    evidence_score: s,
                    follow_on_count: follow,
                    ucb,
                }
            })
            .collect();
        ranked.sort_by(|a, b| {
            b.evidence_score
                .cmp(&a.evidence_score)
                .then_with(|| b.ucb.total_cmp(&a.ucb))
        });
        let leaders = ranked.iter().take(5).cloned().collect();
        let neglected_leaves = ranked
            .iter()
            .filter(|r| {
                r.follow_on_count == 0
                    && !matches!(
                        r.contribution.kind,
                        ContributionKind::Wip | ContributionKind::Endorsed
                    )
            })
            .take(5)
            .cloned()
            .collect();
        let open_hypotheses = ranked
            .iter()
            .filter(|r| {
                r.contribution.kind == ContributionKind::Hypothesis && r.follow_on_count == 0
            })
            .cloned()
            .collect();
        let verified: HashSet<_> = latest_verdict.keys().map(|(_, t)| *t).collect();
        let unverified_results = ranked
            .iter()
            .filter(|r| {
                r.contribution.kind == ContributionKind::Result
                    && !verified.contains(&r.contribution.id)
            })
            .cloned()
            .collect();
        let mut verdicts: HashMap<Uuid, HashSet<VerificationVerdict>> = HashMap::new();
        for ((_, t), v) in latest_verdict {
            if let Some(x) = v.verdict {
                verdicts.entry(t).or_default().insert(x);
            }
        }
        let contested_targets = verdicts
            .into_iter()
            .filter_map(|(t, v)| (v.len() > 1).then_some(t))
            .collect();
        Ok(ResearchAnalysis {
            total: nodes.len(),
            leaders,
            neglected_leaves,
            open_hypotheses,
            unverified_results,
            contested_targets,
        })
    }
}
