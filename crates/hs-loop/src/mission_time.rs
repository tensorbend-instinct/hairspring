//! `T_mission` decomposition (v5 spec "The mission-time model", checklist
//! 7.9). Mission time decomposes into terms the system measures from its
//! OWN substrate log:
//!
//! ```text
//! T_mission = N_steps * (t_model + t_overhead)   // the hot path
//!           + R_failures * t_recover             // what resume/snapshots attack
//!           + C_coord                            // coordination instances
//!           + S_stuck                            // unproductive loops
//! ```
//!
//! Every term here is measured, never estimated: model latency and prompt
//! assembly are booked on every `ModelCall` event, coordination events are
//! counted by kind, and stuck loops are the same normalized-signature
//! duplicate detection the loop's doom-loop telemetry uses, priced at the
//! model step that produced each wasted call. The report is published per
//! run, win or lose (spec gate 8).

use hs_core::{Event, EventKind};
use hs_log::{LogError, StreamReader};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One mission's measured time decomposition. All times in milliseconds.
#[derive(Clone, Debug, Default)]
pub struct Decomposition {
    pub n_steps: u64,
    pub t_model_ms: u64,
    pub t_overhead_ms: u64,
    pub r_failures: u64,
    pub t_recover_ms: u64,
    pub c_coord_events: u64,
    pub c_coord_ms: u64,
    pub s_stuck_repeats: u64,
    pub s_stuck_ms: u64,
    pub wall_ms: i64,
    /// Wall time not attributed to any term (feedback execution, plugin
    /// latency outside model calls, harness bookkeeping outside assembly).
    pub unattributed_ms: i64,
}

impl Decomposition {
    /// Hot path: `N_steps * (t_model + t_overhead)` in its summed form.
    #[must_use]
    pub fn hot_path_ms(&self) -> u64 {
        self.t_model_ms + self.t_overhead_ms
    }
    /// All measured terms summed.
    #[must_use]
    pub fn terms_ms(&self) -> u64 {
        self.hot_path_ms() + self.t_recover_ms + self.c_coord_ms + self.s_stuck_ms
    }
    fn avg(&self, total: u64) -> f64 {
        if self.n_steps == 0 {
            0.0
        } else {
            total as f64 / self.n_steps as f64
        }
    }
    /// The published report body (checklist 7.9 / gate 8: report the full
    /// decomposition per run, win or lose).
    #[must_use]
    pub fn report_lines(&self, mission: &str) -> Vec<String> {
        vec![
            "T_mission decomposition (measured from the mission's own event log, published per run, win or lose)".to_string(),
            format!("mission={mission}"),
            format!("N_steps={}", self.n_steps),
            format!(
                "t_model_total_ms={} avg_ms_per_step={:.1}",
                self.t_model_ms,
                self.avg(self.t_model_ms)
            ),
            format!(
                "t_overhead_total_ms={} avg_ms_per_step={:.1}",
                self.t_overhead_ms,
                self.avg(self.t_overhead_ms)
            ),
            format!(
                "hot_path_ms={} (= N_steps * (t_model + t_overhead))",
                self.hot_path_ms()
            ),
            format!(
                "R_failures={} t_recover_total_ms={}",
                self.r_failures, self.t_recover_ms
            ),
            format!(
                "C_coord events={} ms={}",
                self.c_coord_events, self.c_coord_ms
            ),
            format!(
                "S_stuck repeats={} ms={} (normalized-signature duplicate tool calls beyond the first, each priced at the model step that produced it)",
                self.s_stuck_repeats, self.s_stuck_ms
            ),
            format!("terms_total_ms={}", self.terms_ms()),
            format!(
                "wall_ms={} unattributed_ms={}",
                self.wall_ms, self.unattributed_ms
            ),
        ]
    }
    /// Write the decomposition artifact and return its path.
    pub fn write_report(&self, dir: &Path, mission: &str) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let safe: String = mission
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
            .collect();
        let path = dir.join(format!("t-mission-{safe}.txt"));
        let mut body = self.report_lines(mission).join("\n");
        body.push('\n');
        std::fs::write(&path, body)?;
        Ok(path)
    }
}

pub struct MissionTime;

impl MissionTime {
    /// Decompose one mission's stream. The reader resolves payloads, so
    /// blob-stored events measure exactly like inline ones.
    pub fn decompose(reader: &StreamReader) -> Result<Decomposition, LogError> {
        let events = reader.events()?;
        let mut d = Decomposition::default();
        if events.is_empty() {
            return Ok(d);
        }
        d.wall_ms = events.last().map_or(0, |l| l.ts_wall_ms) - events[0].ts_wall_ms;

        // price of the most recent model step (latency + assembly): the
        // cost a stuck duplicate call wastes
        let mut last_step_ms: u64 = 0;
        let mut seen: HashMap<String, u32> = HashMap::new();
        for ev in &events {
            let text = reader
                .resolve_payload(ev)
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();
            match ev.kind {
                EventKind::ModelCall => {
                    d.n_steps += 1;
                    d.t_model_ms += u64::from(ev.latency_ms);
                    let asm = serde_json::from_str::<Value>(&text)
                        .ok()
                        .and_then(|v| v.get("assembly_ms").and_then(Value::as_u64))
                        .unwrap_or(0);
                    d.t_overhead_ms += asm;
                    last_step_ms = u64::from(ev.latency_ms) + asm;
                }
                EventKind::Message | EventKind::Proposal | EventKind::Consequence => {
                    d.c_coord_events += 1;
                    d.c_coord_ms += u64::from(ev.latency_ms);
                }
                EventKind::Observation | EventKind::Feedback => {
                    // Recovery bookings (tiers A/B/C) carry plain text with
                    // the recovered duration; none booked yet on this path -
                    // the term reads honestly as zero until they exist.
                    if text.contains("recovery") || text.contains("resume tier") {
                        d.r_failures += 1;
                        d.t_recover_ms += parse_ms(&text);
                    }
                }
                EventKind::ToolCall => {
                    if let Some(sig) = tool_signature(&text) {
                        let c = seen.entry(sig).or_insert(0);
                        if *c >= 1 {
                            // a repeated identical call is an unproductive
                            // loop step; price it at its producing model step
                            d.s_stuck_repeats += 1;
                            d.s_stuck_ms += last_step_ms;
                        }
                        *c += 1;
                    }
                }
                _ => {}
            }
        }
        let terms = i64::try_from(d.terms_ms()).unwrap_or(i64::MAX);
        d.unattributed_ms = d.wall_ms - terms;
        Ok(d)
    }
}

/// First bare integer immediately followed by "ms" (recovery bookings).
fn parse_ms(text: &str) -> u64 {
    let mut best = 0u64;
    let bytes = text.as_bytes();
    let mut acc = 0u64;
    let mut any = false;
    for (i, &b) in bytes.iter().enumerate() {
        if b.is_ascii_digit() {
            acc = acc.saturating_mul(10).saturating_add(u64::from(b - b'0'));
            any = true;
        } else {
            if any
                && bytes[i..].starts_with(b"ms")
                && (i + 2 == bytes.len() || !bytes[i + 2].is_ascii_alphanumeric())
            {
                best = acc;
            }
            acc = 0;
            any = false;
        }
    }
    best
}

/// Normalized (plugin, args) signature: identical to the ledger's
/// doom-loop hashing rule (whitespace-insensitive), so telemetry and the
/// decomposition agree on what "the same call twice" means.
fn tool_signature(payload: &str) -> Option<String> {
    let v: Value = serde_json::from_str(payload).ok()?;
    let plugin = v.get("plugin")?.as_str()?;
    let args = v.get("args")?;
    let mut norm = String::new();
    crate::ledger::normalize_strings(args, &mut norm);
    Some(format!("{plugin}|{norm}"))
}

// keep Event referenced for the public surface docs; decompose works on
// resolved payloads via the reader
#[allow(dead_code)]
fn _event_anchor(_: &Event) {}
