use hs_core::{EventBuilder,EventKind,Payload};
use serde_json::json;

#[test]
fn real_kernel_export_counts_once_and_pairs_errors() {
    let dir=tempfile::tempdir().unwrap();
    let log=tempfile::tempdir().unwrap();
    let root=uuid::Uuid::new_v4();
    let mut w=hs_log::StreamWriter::create(log.path(),root).unwrap();
    let mission=w.append(EventBuilder::new(EventKind::Observation).payload(Payload::Inline(
        serde_json::to_vec(&json!({"record_type":"mission_start"})).unwrap()))).unwrap();
    let plugin=env!("CARGO_BIN_EXE_hs-plugin-fakemodel");
    let cfg=dir.path().join("hs.toml");
    std::fs::write(&cfg,format!("[[models]]\nname=\"fake-v1\"\ncommand=[\"{plugin}\"]\ndefault=true\n")).unwrap();
    let kernel=hs_kernel::Kernel::load_with_log(&cfg,log.path()).unwrap();
    kernel.set_trace_parent(mission.event_id);
    let out=kernel.call_model("operator",None,"hello").unwrap();
    w.append(EventBuilder::new(EventKind::ModelCall).parent(out.start_event_id).payload(Payload::Inline(
        serde_json::to_vec(&json!({"record_type":"mirror","call_id":out.call_id,"cost_usd_micros":out.cost_usd_micros})).unwrap()))).unwrap();
    let mut records=vec![];
    for sid in [root,kernel.stream_id().unwrap()] {
        let r=hs_log::StreamReader::open(log.path(),sid).unwrap();
        hs_log::verify_stream(log.path(),sid).unwrap();
        for e in r.events().unwrap() {let v=serde_json::from_slice(&r.resolve_payload(&e).unwrap()).unwrap();records.push((e,v));}
    }
    let (payload,audit)=hs_cli::otlp::export(&records);
    assert_eq!(audit["logical_model_calls"],1);
    assert_eq!(audit["mirror_records_ignored"],1);
    assert_eq!(audit["cost_usd_micros"],out.cost_usd_micros);
    assert_eq!(audit["orphan_links"],0);
    assert_eq!(payload["resourceSpans"][0]["scopeSpans"][0]["spans"].as_array().unwrap().len(),2);
    std::fs::write("/tmp/hairspring-otlp-validation.json",serde_json::to_vec_pretty(&payload).unwrap()).unwrap();
}

#[test]
fn unmatched_start_and_legacy_call_are_not_success() {
    let dir=tempfile::tempdir().unwrap();
    let sid=uuid::Uuid::new_v4();
    let mut w=hs_log::StreamWriter::create(dir.path(),sid).unwrap();
    let id=uuid::Uuid::new_v4();
    let start=w.append(EventBuilder::new(EventKind::Observation).payload(Payload::Inline(
        serde_json::to_vec(&json!({"record_type":"start","call_id":id,"method":"model.call"})).unwrap()))).unwrap();
    let legacy=w.append(EventBuilder::new(EventKind::ModelCall)).unwrap();
    let (p,a)=hs_cli::otlp::export(&[(start,json!({"record_type":"start","call_id":id,"method":"model.call"})),(legacy,json!({}))]);
    assert_eq!(a["logical_model_calls"],0);
    assert_eq!(a["incomplete_call_ids"].as_array().unwrap().len(),1);
    assert_eq!(a["legacy_unidentifiable_calls"],1);
    assert!(p["resourceSpans"][0]["scopeSpans"][0]["spans"].as_array().unwrap().is_empty());
}
