//! ATIF (Agent Trajectory Interchange Format, Harbor RFC 0001, v1.8) export.
//! One trajectory per mission_start root. Each paired model.call becomes one
//! agent step (llm_call_count=1); tool calls that finish before the next model
//! call attach to the preceding agent step as tool_calls plus observation
//! results keyed by source_call_id. Unpaired or duplicate-identity calls are
//! reported in the audit and never invented as steps. Cost is
//! cost_usd_micros / 1e6 as recorded; no pricing is recomputed.
use hs_core::Event;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub const SCHEMA_VERSION: &str = "ATIF-v1.8";

fn iso(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let sub = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{sub:03}Z", rem / 3600, (rem % 3600) / 60, rem % 60)
}

fn text(v: &Value) -> String {
    match v { Value::String(s) => s.clone(), Value::Null => String::new(), o => o.to_string() }
}

/// Returns (ATIF trajectories, audit). Caller verifies chains and resolves blobs.
pub fn export(records: &[(Event, Value)], agent_version: &str) -> (Vec<Value>, Value) {
    let mut roots: Vec<&(Event, Value)> = Vec::new();
    let mut starts: HashMap<String, &(Event, Value)> = HashMap::new();
    let mut ends: HashMap<String, &(Event, Value)> = HashMap::new();
    let mut conflicted: HashSet<String> = HashSet::new();
    for r in records {
        let id = r.1["call_id"].as_str().unwrap_or("").to_string();
        match r.1["record_type"].as_str() {
            Some("mission_start") => roots.push(r),
            Some("start") => { if starts.insert(id.clone(), r).is_some() { conflicted.insert(id); } }
            Some("end") => { if ends.insert(id.clone(), r).is_some() { conflicted.insert(id); } }
            _ => {}
        }
    }
    let by_event: HashMap<_, _> = starts.values().map(|r| (r.0.event_id, *r)).collect();
    let root_ids: HashSet<_> = roots.iter().map(|r| r.0.event_id).collect();
    let root_of = |e: &Event| {
        let mut cur = e.parent_event_id;
        for _ in 0..128 {
            let Some(p) = cur else { return None };
            if root_ids.contains(&p) { return Some(p); }
            cur = by_event.get(&p).and_then(|r| r.0.parent_event_id);
        }
        None
    };
    let mut incomplete = Vec::new();
    let mut unrooted = 0;
    // (root, start_ts, seq, is_model, call_id)
    let mut calls: Vec<(uuid::Uuid, i64, u64, bool, String)> = Vec::new();
    for (id, s) in &starts {
        if conflicted.contains(id) { continue; }
        let Some(e) = ends.get(id) else { incomplete.push(id.clone()); continue };
        if e.0.parent_event_id != Some(s.0.event_id) { continue; }
        let Some(root) = root_of(&s.0) else { unrooted += 1; continue };
        let method = s.1["method"].as_str().unwrap_or("");
        if method == "model.call" || method == "tool.call" {
            calls.push((root, s.0.ts_wall_ms, s.0.seq, method == "model.call", id.clone()));
        }
    }
    calls.sort_by_key(|c| (c.1, c.2));
    let mut out = Vec::new();
    for root in &roots {
        let mission = root.1["mission"].as_str().unwrap_or("mission");
        let goal = root.1["goal"].as_str().unwrap_or(mission);
        let mut steps = vec![json!({"step_id":1,"timestamp":iso(root.0.ts_wall_ms),"source":"user","message":goal})];
        let (mut pt, mut ct, mut cached, mut cost) = (0u64, 0u64, 0u64, 0i64);
        for c in calls.iter().filter(|c| c.0 == root.0.event_id) {
            let (s, e) = (starts[&c.4], ends[&c.4]);
            if c.3 {
                let ev = &e.1;
                let ok = ev["status"] != "error";
                let (i, o, k) = (ev["input_tokens"].as_u64().unwrap_or(0), ev["output_tokens"].as_u64().unwrap_or(0), ev["cached_tokens"].as_u64().unwrap_or(0));
                let micros = e.0.cost_usd_micros.max(0);
                let mut step = json!({"step_id":steps.len()+1,"timestamp":iso(s.0.ts_wall_ms),"source":"agent",
                    "model_name":ev["model"],"message":if ok {text(&ev["completion"])} else {format!("ERROR: {}", text(&ev["error"]))},
                    "llm_call_count":1,
                    "metrics":{"prompt_tokens":i,"completion_tokens":o,"cached_tokens":k,"cost_usd":micros as f64/1e6,
                        "extra":{"reasoning_tokens":ev["reasoning_tokens"].as_u64().unwrap_or(0),"call_id":c.4}}});
                if let Some(r) = ev["reasoning_content"].as_str().filter(|r| !r.is_empty()) { step["reasoning_content"] = json!(r); }
                steps.push(step);
                pt += i; ct += o; cached += k; cost += micros;
            } else if let Some(step) = steps.last_mut().filter(|s| s["source"] == "agent") {
                let name = s.1["plugin"].as_str().unwrap_or("");
                let args = if s.1["args"].is_object() { s.1["args"].clone() } else { json!({}) };
                if !step["tool_calls"].is_array() { step["tool_calls"] = json!([]); }
                step["tool_calls"].as_array_mut().unwrap().push(json!({"tool_call_id":c.4,"function_name":name,"arguments":args}));
                if step["observation"].is_null() { step["observation"] = json!({"results":[]}); }
                let content = if e.1["status"] == "error" { format!("ERROR: {}", text(&e.1["error"])) } else { text(&e.1["result"]) };
                step["observation"]["results"].as_array_mut().unwrap().push(json!({"source_call_id":c.4,"content":content}));
            }
        }
        let n = steps.len();
        out.push(json!({"schema_version":SCHEMA_VERSION,"session_id":mission,"trajectory_id":root.0.event_id.to_string(),
            "agent":{"name":"hairspring","version":agent_version},"steps":steps,
            "final_metrics":{"total_prompt_tokens":pt,"total_completion_tokens":ct,"total_cached_tokens":cached,
                "total_cost_usd":cost as f64/1e6,"total_steps":n}}));
    }
    let audit = json!({"trajectories":out.len(),"incomplete_call_ids":incomplete,
        "duplicate_identity_conflicts":conflicted.len(),"calls_without_mission_root":unrooted});
    (out, audit)
}
