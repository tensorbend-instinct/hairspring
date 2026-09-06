//! D2 execution ledger: a read model over the mission's ToolCall events,
//! updated per event. The assembler (D1) always includes its summary; exact
//! duplicate (tool, args) calls are flagged against `last_calls` with the
//! prior seq, converting silent re-read loops into an explicit signal (P3).

use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

/// Hard cap on the rendered summary: ~2k tokens at 4 chars/token (T8).
pub const LEDGER_SUMMARY_CHARS: usize = 8000;

const LAST_CALLS_CAP: usize = 512;
const RANGES_PER_FILE_STORED: usize = 16;
const RANGES_PER_FILE_SHOWN: usize = 3;
const EDITS_SHOWN: usize = 10;
const TESTS_SHOWN: usize = 10;
const OPEN_SHOWN: usize = 5;

#[derive(Default)]
pub struct Ledger {
    files_read: BTreeMap<String, Vec<(u32, u32)>>,
    edits: Vec<(u64, String)>,
    test_runs: Vec<(u64, String, bool, String)>,
    /// seq watermark of the last verifier-mandated workspace restore:
    /// runs recorded at or before it describe a tree that no longer
    /// exists and render marked (feedback integrity F8).
    restored_before: Option<u64>,
    open_threads: Vec<String>,
    last_calls: VecDeque<(String, u64, u64, u64)>, // (plugin, args_hash, args_hash_normalized, seq)
}

fn args_hash(args: &Value) -> u64 {
    let mut h = DefaultHasher::new();
    h.write(args.to_string().as_bytes());
    h.finish()
}

/// Doom-loop hashing ignores whitespace-only differences: "sh check.sh" and
/// "sh  check.sh" are the same stuck call (Grok doom_loop_telemetry,
/// adapted).
fn normalize_strings(v: &Value, out: &mut String) {
    match v {
        Value::String(s) => {
            out.push('"');
            out.push_str(&s.split_whitespace().collect::<Vec<_>>().join(" "));
            out.push('"');
        }
        Value::Array(a) => {
            out.push('[');
            for x in a {
                normalize_strings(x, out);
            }
            out.push(']');
        }
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for k in keys {
                out.push_str(k);
                out.push(':');
                normalize_strings(&m[k], out);
            }
            out.push('}');
        }
        other => out.push_str(&other.to_string()),
    }
}

fn args_hash_normalized(args: &Value) -> u64 {
    let mut norm = String::new();
    normalize_strings(args, &mut norm);
    let mut h = DefaultHasher::new();
    h.write(norm.as_bytes());
    h.finish()
}

fn merge_range(ranges: &mut Vec<(u32, u32)>, lo: u32, hi: u32) {
    for r in ranges.iter_mut() {
        if lo <= r.1.saturating_add(1) && hi + 1 >= r.0 {
            r.0 = r.0.min(lo);
            r.1 = r.1.max(hi);
            return;
        }
    }
    if ranges.len() < RANGES_PER_FILE_STORED {
        ranges.push((lo, hi));
    }
}

/// Bounded output tail for a recorded run (F7): the verifier audits
/// recorded evidence, and a run rendered without its output is
/// indistinguishable from a bare claim. Whitespace-collapsed, last 160
/// chars (the verdict line lives at the tail for test runners).
fn output_tail(result: &Value) -> String {
    let flat = result["stdout"]
        .as_str()
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    flat.chars().rev().take(160).collect::<Vec<_>>().into_iter().rev().collect()
}

impl Ledger {
    /// Fold one ToolCall event into the projection.
    pub fn apply_tool_call(&mut self, seq: u64, plugin: &str, args: &Value, result: &Value) {
        if self.last_calls.len() >= LAST_CALLS_CAP {
            self.last_calls.pop_front();
        }
        self.last_calls.push_back((plugin.to_string(), args_hash(args), args_hash_normalized(args), seq));

        match plugin {
            "repo.read" => {
                if let Some(path) = args["path"].as_str() {
                    let start = args["start_line"].as_u64().unwrap_or(1) as u32;
                    let n = args["max_lines"].as_u64().unwrap_or(400) as u32;
                    let total = result["total_lines"].as_u64().unwrap_or(0) as u32;
                    let _ = total;
                    merge_range(
                        self.files_read.entry(path.to_string()).or_default(),
                        start,
                        start.saturating_add(n).saturating_sub(1),
                    );
                }
            }
            "edit.apply" => {
                if result["applied"].as_bool() == Some(true) {
                    if let Some(files) = result["files_changed"].as_array() {
                        for f in files.iter().filter_map(|f| f.as_str()) {
                            self.edits.push((seq, f.to_string()));
                        }
                    }
                }
            }
            "answer.write" => {
                if let Some(p) = args["path"].as_str() {
                    self.edits.push((seq, p.to_string()));
                }
            }
            "repo.exec" => {
                if result["applied"].as_bool() == Some(true) {
                    let cmd = args["command"].as_str().unwrap_or("").chars().take(60).collect();
                    let ok = result["exit_code"].as_i64() == Some(0);
                    self.test_runs.push((seq, cmd, ok, output_tail(result)));
                }
            }
            "checker.run" => {
                let ok = result["passed"].as_bool().unwrap_or(false);
                self.test_runs.push((seq, "checker".to_string(), ok, String::new()));
            }
            "notes.scratch" => {
                if matches!(args["op"].as_str(), Some("write") | Some("append")) {
                    if let Some(line) = args["content"].as_str()
                        .and_then(|c| c.lines().find(|l| !l.trim().is_empty()))
                    {
                        self.open_threads.push(line.chars().take(80).collect());
                        if self.open_threads.len() > OPEN_SHOWN {
                            self.open_threads.remove(0);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// True once the MODEL has verified something itself (a repo.exec run).
    /// The harness's own checker.run verdicts are ground truth, not the
    /// model testing its work, so they never count (fix 5).
    pub fn model_verified(&self) -> bool {
        self.test_runs.iter().any(|(_, cmd, _, _)| cmd != "checker")
    }

    /// Mark every run recorded at or before `seq` as pre-restore (F8): a
    /// refuted verdict restored the workspace to the audited snapshot, so
    /// those runs describe a tree that no longer exists. Display-only;
    /// the history itself is kept.
    pub fn note_restore(&mut self, seq: u64) {
        self.restored_before = Some(seq);
    }

    /// Latest recorded call seq (0 when the ledger is empty).
    pub fn last_seq(&self) -> u64 {
        self.last_calls.back().map(|t| t.3).unwrap_or(0)
    }

    /// A prior seq for an identical (plugin, args) call, if one exists.
    pub fn find_duplicate(&self, plugin: &str, args: &Value) -> Option<u64> {
        let h = args_hash(args);
        self.last_calls
            .iter()
            .rev()
            .find(|(p, ah, _, _)| p == plugin && *ah == h)
            .map(|(_, _, _, seq)| *seq)
    }

    /// Doom-loop detection (item 4, Grok doom_loop_telemetry adapted):
    /// within the last `window` calls, count the largest group sharing
    /// (plugin, whitespace-normalized args). Returns (plugin, count) when
    /// that count reaches `threshold` - the caller owns fire-once and
    /// escalation policy.
    pub fn doom_loop_repeat(&self, window: usize, threshold: usize) -> Option<(String, usize)> {
        let mut best: Option<(String, usize)> = None;
        let n = self.last_calls.len();
        for (p, _, nh, _) in self.last_calls.iter().skip(n.saturating_sub(window)) {
            let count = self
                .last_calls
                .iter()
                .skip(n.saturating_sub(window))
                .filter(|(p2, _, nh2, _)| p2 == p && nh2 == nh)
                .count();
            if count >= threshold && best.as_ref().map(|(_, c)| count > *c).unwrap_or(true) {
                best = Some((p.clone(), count));
            }
        }
        best
    }

    /// Bounded render for the always-resident LEDGER prompt block (T8).
    pub fn summary(&self) -> String {
        let mut s = String::new();
        if !self.files_read.is_empty() {
            s.push_str("reads: ");
            let total = self.files_read.len();
            let mut shown = 0usize;
            for (path, ranges) in self.files_read.iter().rev() {
                if s.len() > LEDGER_SUMMARY_CHARS / 2 || shown >= 30 {
                    break;
                }
                let rs: Vec<String> = ranges
                    .iter()
                    .take(RANGES_PER_FILE_SHOWN)
                    .map(|(a, b)| format!("L{a}-{b}"))
                    .collect();
                let more = if ranges.len() > RANGES_PER_FILE_SHOWN {
                    format!("+{}", ranges.len() - RANGES_PER_FILE_SHOWN)
                } else {
                    String::new()
                };
                s.push_str(&format!("{path}:{}{more}; ", rs.join(",")));
                shown += 1;
            }
            if total > shown {
                s.push_str(&format!("(+{} more files) ", total - shown));
            }
            s.push('\n');
        }
        if !self.edits.is_empty() {
            s.push_str("edits: ");
            for (seq, path) in self.edits.iter().rev().take(EDITS_SHOWN).rev() {
                s.push_str(&format!("{path}@seq{seq}; "));
            }
            s.push('\n');
        }
        if !self.model_verified() && (!self.files_read.is_empty() || !self.edits.is_empty()) {
            s.push_str("tests: NO TEST RUN YET - you have not verified anything yourself this mission
");
        }
        if !self.test_runs.is_empty() {
            s.push_str("tests: ");
            for (seq, cmd, ok, tail) in self.test_runs.iter().rev().take(TESTS_SHOWN).rev() {
                let stale = if self.restored_before.map(|r| *seq <= r).unwrap_or(false) { "(pre-restore)" } else { "" };
                let ev_tail = if tail.is_empty() { String::new() } else { format!(" [{tail}]") };
                s.push_str(&format!("\"{cmd}\" {}@seq{seq}{stale}{ev_tail}; ", if *ok { "PASS" } else { "FAIL" }));
            }
            s.push('\n');
        }
        if !self.open_threads.is_empty() {
            s.push_str("notes: ");
            for t in &self.open_threads {
                s.push_str(&format!("- {t} "));
            }
            s.push('\n');
        }
        if s.is_empty() {
            s.push_str("(no tool calls yet)\n");
        }
        if s.len() > LEDGER_SUMMARY_CHARS {
            s.truncate(LEDGER_SUMMARY_CHARS - 20);
            s.push_str("...[ledger capped]");
        }
        s
    }
}
