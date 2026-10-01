use hs_core::{EventBuilder, EventKind, Payload};
use serde_json::{json, Value};

fn rec(w: &mut hs_log::StreamWriter, kind: EventKind, parent: Option<uuid::Uuid>, ts: i64, cost: i64, v: Value) -> hs_core::Event {
    let mut b = EventBuilder::new(kind).payload(Payload::Inline(serde_json::to_vec(&v).unwrap())).cost_usd_micros(cost);
    if let Some(p) = parent { b = b.parent(p); }
    w.append(b.ts_wall_ms(ts)).unwrap()
}

#[test]
fn trajectory_has_required_atif_fields_and_correlated_tool_results() {
    let d = tempfile::tempdir().unwrap();
    let sid = uuid::Uuid::new_v4();
    let mut w = hs_log::StreamWriter::create(d.path(), sid).unwrap();
    let root = rec(&mut w, EventKind::Observation, None, 1_700_000_000_000, 0, json!({"record_type":"mission_start","mission":"m1","goal":"fix the bug"}));
    let (m, t) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
    let ms = rec(&mut w, EventKind::Observation, Some(root.event_id), 1_700_000_001_000, 0, json!({"record_type":"start","call_id":m,"method":"model.call","plugin":"deepseek"}));
    rec(&mut w, EventKind::ModelCall, Some(ms.event_id), 1_700_000_002_000, 800, json!({"record_type":"end","call_id":m,"status":"ok","model":"deepseek","completion":"run tests","input_tokens":100,"output_tokens":20,"cached_tokens":40,"reasoning_tokens":5,"reasoning_content":"think"}));
    let ts = rec(&mut w, EventKind::Observation, Some(root.event_id), 1_700_000_003_000, 0, json!({"record_type":"start","call_id":t,"method":"tool.call","plugin":"repo.exec","args":{"command":"cargo test"}}));
    rec(&mut w, EventKind::ToolCall, Some(ts.event_id), 1_700_000_004_000, 0, json!({"record_type":"end","call_id":t,"status":"ok","plugin":"repo.exec","result":{"exit_code":0}}));
    let r = hs_log::StreamReader::open(d.path(), sid).unwrap();
    let recs: Vec<_> = r.events().unwrap().into_iter().map(|e| { let v = serde_json::from_slice(&r.resolve_payload(&e).unwrap()).unwrap(); (e, v) }).collect();
    let (trajs, audit) = hs_cli::atif::export(&recs, "0.1.0");
    assert_eq!(audit["trajectories"], 1);
    let t0 = &trajs[0];
    assert_eq!(t0["schema_version"], "ATIF-v1.8");
    assert_eq!(t0["agent"]["name"], "hairspring");
    assert_eq!(t0["agent"]["version"], "0.1.0");
    let steps = t0["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0]["step_id"], 1);
    assert_eq!(steps[0]["source"], "user");
    assert_eq!(steps[0]["message"], "fix the bug");
    assert_eq!(steps[0]["timestamp"], "2023-11-14T22:13:20.000Z");
    let a = &steps[1];
    assert_eq!(a["step_id"], 2);
    assert_eq!(a["source"], "agent");
    assert_eq!(a["message"], "run tests");
    assert_eq!(a["reasoning_content"], "think");
    assert_eq!(a["llm_call_count"], 1);
    assert_eq!(a["metrics"]["prompt_tokens"], 100);
    assert_eq!(a["metrics"]["cached_tokens"], 40);
    assert_eq!(a["metrics"]["cost_usd"], 0.0008);
    let tc = &a["tool_calls"][0];
    assert_eq!(tc["tool_call_id"], t.to_string());
    assert_eq!(tc["function_name"], "repo.exec");
    assert_eq!(tc["arguments"]["command"], "cargo test");
    assert_eq!(a["observation"]["results"][0]["source_call_id"], tc["tool_call_id"]);
    assert_eq!(t0["final_metrics"]["total_prompt_tokens"], 100);
    assert_eq!(t0["final_metrics"]["total_cost_usd"], 0.0008);
    assert_eq!(t0["final_metrics"]["total_steps"], 2);
}

#[test]
fn unpaired_model_call_is_reported_not_exported() {
    let d = tempfile::tempdir().unwrap();
    let sid = uuid::Uuid::new_v4();
    let mut w = hs_log::StreamWriter::create(d.path(), sid).unwrap();
    let root = rec(&mut w, EventKind::Observation, None, 1_700_000_000_000, 0, json!({"record_type":"mission_start","mission":"m2"}));
    let m = uuid::Uuid::new_v4();
    rec(&mut w, EventKind::Observation, Some(root.event_id), 1_700_000_001_000, 0, json!({"record_type":"start","call_id":m,"method":"model.call"}));
    let r = hs_log::StreamReader::open(d.path(), sid).unwrap();
    let recs: Vec<_> = r.events().unwrap().into_iter().map(|e| { let v = serde_json::from_slice(&r.resolve_payload(&e).unwrap()).unwrap(); (e, v) }).collect();
    let (trajs, audit) = hs_cli::atif::export(&recs, "0.1.0");
    assert_eq!(trajs[0]["steps"].as_array().unwrap().len(), 1);
    assert_eq!(audit["incomplete_call_ids"].as_array().unwrap().len(), 1);
}
