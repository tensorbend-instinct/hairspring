//! D7 evolution loop (AVO pattern): candidate prompts are population
//! members with lineage refs. A sandbox bench runs parent and candidate on
//! a task subset through REAL missions; promotion requires beating the
//! parent on HELD-OUT tasks; rejections are recorded with reasons and feed
//! the next mutation; rewind restores the parent (exo rollback). The runner
//! is injected so the same driver serves fixture benches and the SWE path.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct BenchOutcome {
    pub task: String,
    pub passed: bool,
    pub steps: u32,
    pub cost_micros: u64,
    pub stream_id: uuid::Uuid,
}

#[derive(Debug, PartialEq)]
pub enum Decision {
    Promoted,
    Rejected(String),
}

#[derive(Debug)]
pub struct Evaluation {
    pub decision: Decision,
    pub parent_hash: String,
    pub candidate_hash: String,
    pub bench: Vec<BenchOutcome>,
    pub held_out_parent: Vec<BenchOutcome>,
    pub held_out_candidate: Vec<BenchOutcome>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JournalRecord {
    version: u64,
    decision: String, // promoted | rejected | rewound
    reason: String,
    parent_hash: String,
    candidate_hash: String,
    parent_text: Option<String>,
    candidate_text: Option<String>,
    trace_streams: Vec<String>,
    ts_ms: i64,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn read_journal(path: &Path) -> Vec<JournalRecord> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn append_journal(path: &Path, rec: &JournalRecord) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("journal open");
    writeln!(
        f,
        "{}",
        serde_json::to_string(rec).expect("bench records serialize")
    )
    .expect("journal write");
}

fn fitness(outcomes: &[BenchOutcome]) -> (u32, u64) {
    // (passes, total steps): passes dominate, fewer steps break ties
    (
        outcomes.iter().filter(|o| o.passed).count() as u32,
        outcomes.iter().map(|o| u64::from(o.steps)).sum(),
    )
}

fn write_overlay(path: &Path, template: &str) {
    std::fs::write(
        path,
        format!(
            "[prompts]\nswe-mission = {}\n",
            toml::Value::String(template.to_string())
        ),
    )
    .expect("overlay write");
}

/// Run parent and candidate through the bench subset, decide on held-out.
/// `runner` builds and runs ONE real mission for (template, task): None =
/// the builtin template. Promotion writes the policy overlay; every outcome
/// journaled with lineage + trace stream refs.
pub fn evaluate_candidate(
    runner: &dyn Fn(Option<&str>, &str) -> BenchOutcome,
    parent_text: Option<String>,
    candidate_text: String,
    bench: &[String],
    held_out: &[String],
    overlay_path: &Path,
    journal_path: &Path,
) -> Evaluation {
    let parent_hash = parent_text
        .as_deref().map_or_else(|| "builtin".to_string(), crate::sweprompt::content_hash);
    let candidate_hash = crate::sweprompt::content_hash(&candidate_text);

    let bench_out: Vec<BenchOutcome> = bench
        .iter()
        .map(|t| runner(Some(&candidate_text), t))
        .collect();
    let ho_parent: Vec<BenchOutcome> = held_out
        .iter()
        .map(|t| runner(parent_text.as_deref(), t))
        .collect();
    let ho_candidate: Vec<BenchOutcome> = held_out
        .iter()
        .map(|t| runner(Some(&candidate_text), t))
        .collect();

    let (pp, ps) = fitness(&ho_parent);
    let (cp, cs) = fitness(&ho_candidate);
    let beats = cp > pp || (cp == pp && cs < ps);
    let traces: Vec<String> = bench_out
        .iter()
        .chain(&ho_parent)
        .chain(&ho_candidate)
        .map(|o| o.stream_id.to_string())
        .collect();

    let version = read_journal(journal_path).len() as u64 + 1;
    let (decision, reason) = if beats {
        write_overlay(overlay_path, &candidate_text);
        (
            Decision::Promoted,
            format!("candidate beats parent on held-out: passes {cp} vs {pp}, steps {cs} vs {ps}"),
        )
    } else {
        (Decision::Rejected(format!("candidate does not beat parent on held-out: passes {cp} vs {pp}, steps {cs} vs {ps}")),
         format!("candidate does not beat parent on held-out: passes {cp} vs {pp}, steps {cs} vs {ps}"))
    };
    append_journal(
        journal_path,
        &JournalRecord {
            version,
            decision: if beats {
                "promoted".into()
            } else {
                "rejected".into()
            },
            reason,
            parent_hash: parent_hash.clone(),
            candidate_hash: candidate_hash.clone(),
            parent_text: parent_text.clone(),
            candidate_text: if beats {
                Some(candidate_text.clone())
            } else {
                None
            },
            trace_streams: traces,
            ts_ms: now_ms(),
        },
    );
    Evaluation {
        decision,
        parent_hash,
        candidate_hash,
        bench: bench_out,
        held_out_parent: ho_parent,
        held_out_candidate: ho_candidate,
    }
}

/// Roll back the last promotion: restore its parent template (or remove the
/// overlay when the parent was the builtin). Recorded in the journal.
pub fn rewind(journal_path: &Path, overlay_path: &Path) -> Result<(), String> {
    let journal = read_journal(journal_path);
    let last_promotion = journal
        .iter()
        .rev()
        .find(|r| r.decision == "promoted")
        .ok_or("no promotion to rewind")?;
    match &last_promotion.parent_text {
        Some(t) => write_overlay(overlay_path, t),
        None => {
            if overlay_path.exists() {
                std::fs::remove_file(overlay_path).map_err(|e| e.to_string())?;
            }
        }
    }
    append_journal(
        journal_path,
        &JournalRecord {
            version: journal.len() as u64 + 1,
            decision: "rewound".into(),
            reason: format!(
                "rewind promotion of candidate {}",
                last_promotion.candidate_hash
            ),
            parent_hash: last_promotion.parent_hash.clone(),
            candidate_hash: String::new(),
            parent_text: last_promotion.parent_text.clone(),
            candidate_text: None,
            trace_streams: vec![],
            ts_ms: now_ms(),
        },
    );
    Ok(())
}
