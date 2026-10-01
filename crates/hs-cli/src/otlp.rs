//! OTLP JSON + OpenInference attributes, one span per logical invocation.
//! Legacy records and unmatched starts are reported, never invented as success.
use hs_core::Event;
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

fn span_id(id: Uuid) -> String { id.simple().to_string()[..16].to_string() }
fn attr(key: &str, value: Value) -> Value {
    let value = if let Some(n) = value.as_u64() { json!({"intValue":n.to_string()}) }
        else { json!({"stringValue":value.as_str().unwrap_or("")}) };
    json!({"key":key,"value":value})
}

/// Returns an OTLP ExportTraceServiceRequest and separate audit diagnostics.
/// Caller must resolve blobs and verify chains before passing the records.
pub fn export(records: &[(Event, Value)]) -> (Value, Value) {
    let mut starts = HashMap::new();
    let mut ends = HashMap::new();
    let mut roots = HashMap::new();
    let mut mirrors = 0;
    let mut legacy = 0;
    let mut conflicts = 0;
    for (e,v) in records {
        match v["record_type"].as_str() {
            Some("mission_start") => { roots.insert(e.event_id,e); },
            Some("start") => { if starts.insert(v["call_id"].as_str().unwrap_or("").to_string(),(e,v)).is_some() {conflicts+=1;} },
            Some("end") => { if ends.insert(v["call_id"].as_str().unwrap_or("").to_string(),(e,v)).is_some() {conflicts+=1;} },
            Some("mirror") => { mirrors+=1; },
            _ if matches!(e.kind, hs_core::EventKind::ModelCall | hs_core::EventKind::ToolCall) => {legacy+=1;},
            _ => (),
        }
    }
    let start_ids: HashMap<_,_> = starts.values().map(|(e,_)|(e.event_id,*e)).collect();
    let root_of = |event: &Event| {
        let mut current = event;
        let mut root = event.stream_id;
        for _ in 0..128 {
            let Some(parent) = current.parent_event_id else {break;};
            if roots.contains_key(&parent) {root=parent; break;}
            let Some(next) = start_ids.get(&parent) else {break;};
            current=next;
        }
        root
    };
    let mut spans = Vec::new();
    let mut incomplete = Vec::new();
    let mut orphans = 0;
    let mut calls = 0;
    let mut cost = 0i64;
    for (id,(start,sv)) in &starts {
        let Some((end,ev)) = ends.get(id) else {incomplete.push(id.clone());continue;};
        if end.parent_event_id != Some(start.event_id) {orphans+=1;continue;}
        let root = root_of(start);
        let method = sv["method"].as_str().unwrap_or("");
        let llm = method == "model.call";
        let mut attrs=vec![attr("openinference.span.kind",json!(if llm {"LLM"} else {"TOOL"})),attr("hairspring.call_id",json!(id))];
        if llm {
            calls+=1;
            cost+=end.cost_usd_micros;
            for (key,field) in [("llm.token_count.prompt","input_tokens"),("llm.token_count.completion","output_tokens"),("llm.token_count.prompt_details.cache_read","cached_tokens"),("llm.token_count.completion_details.reasoning","reasoning_tokens")] {
                if ev[field].is_number() {attrs.push(attr(key,ev[field].clone()));}
            }
            attrs.push(attr("llm.model_name",ev["model"].clone()));
        } else {attrs.push(attr("tool.name",sv["plugin"].clone()));}
        attrs.push(attr("hairspring.cost_usd_micros",json!(end.cost_usd_micros.max(0))));
        let mut span=json!({"traceId":root.simple().to_string(),"spanId":span_id(start.event_id),
            "name":format!("{} {}",method,sv["plugin"].as_str().unwrap_or("")),"kind":1,
            "startTimeUnixNano":((start.ts_wall_ms.max(0) as u64)*1_000_000).to_string(),
            "endTimeUnixNano":((end.ts_wall_ms.max(start.ts_wall_ms) as u64)*1_000_000).to_string(),
            "attributes":attrs,"status":{"code":if ev["status"] == "error" {2} else {1}}});
        if let Some(parent)=start.parent_event_id {
            if roots.contains_key(&parent)||start_ids.contains_key(&parent) {span["parentSpanId"]=json!(span_id(parent));}
            else {orphans+=1;}
        }
        spans.push(span);
    }
    for (id,root) in roots {
        let end = records.iter().filter(|(e,_)| e.stream_id==root.stream_id && e.ts_wall_ms>=root.ts_wall_ms)
            .map(|(e,_)|e.ts_wall_ms).max().unwrap_or(root.ts_wall_ms);
        spans.push(json!({"traceId":id.simple().to_string(),"spanId":span_id(id),"name":"HAIRSPRING mission","kind":1,
            "startTimeUnixNano":(root.ts_wall_ms.max(0) as u64*1_000_000).to_string(),
            "endTimeUnixNano":(end.max(0) as u64*1_000_000).to_string(),
            "attributes":[attr("openinference.span.kind",json!("AGENT"))]}));
    }
    spans.sort_by_key(|v|v["spanId"].as_str().unwrap_or("").to_string());
    let terminal_without_start=ends.keys().filter(|id| !starts.contains_key(*id)).count();
    let audit=json!({"logical_model_calls":calls,"cost_usd_micros":cost,"mirror_records_ignored":mirrors,
        "incomplete_call_ids":incomplete,"terminal_without_start":terminal_without_start,
        "legacy_unidentifiable_calls":legacy,"orphan_links":orphans,"duplicate_identity_conflicts":conflicts});
    (json!({"resourceSpans":[{"resource":{"attributes":[attr("service.name",json!("hairspring"))]},
        "scopeSpans":[{"scope":{"name":"hairspring.otlp","version":"1"},"spans":spans}]}]}),audit)
}
