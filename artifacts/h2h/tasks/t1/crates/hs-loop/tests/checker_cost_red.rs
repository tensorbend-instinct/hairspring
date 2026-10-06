use hs_loop::InnerLoop;
#[test]
fn checker_cost_is_booked_once_and_survives_session_resume() {
 let d=tempfile::tempdir().unwrap();let log=tempfile::tempdir().unwrap();
 let checker=d.path().join("checker.py");
 std::fs::write(&checker,r#"import sys,json
for line in sys.stdin:
 q=json.loads(line)
 if q.get('method')=='describe': r={'name':'checker.run','kind':'tool','subjects':['*'],'schema':{}}
 else:r={'passed':True,'cost_usd_micros':1234,'critic':{'cost_micros':1234,'trace':[]}}
 print(json.dumps({'jsonrpc':'2.0','id':q.get('id'),'result':r}),flush=True)
"#).unwrap();
 let script=d.path().join("script.jsonl");
 std::fs::write(&script, format!("{}\n{}\n",serde_json::json!({"tool":"answer.write","args":{"path":log.path().join("work/task-0/answer.txt"),"content":"TOKEN-0-SECRET"}}),serde_json::json!({"tool":"verdict.submit","args":{"refuted":false,"findings":[],"blocking":"none"}}))).unwrap();
 let config=d.path().join("hs.toml");
 std::fs::write(&config,format!("[[tools]]\nname=\"answer.write\"\ncommand=[\"{}\"]\n[[tools]]\nname=\"checker.run\"\ncommand=[\"python3\",\"{}\"]\n[[models]]\nname=\"scripted\"\ndefault=true\ncommand=[\"/bin/sh\",\"-c\",\"HS_SCRIPTED_PROMPT_AWARE=0 HS_SEQMODEL_SCRIPT={} exec {}\"]\n",env!("CARGO_BIN_EXE_hs-plugin-answer"),checker.display(),script.display(),env!("CARGO_BIN_EXE_hs-plugin-scripted"))).unwrap();
 let kernel=hs_kernel::Kernel::load(&config).unwrap();let mut l=InnerLoop::new(kernel,log.path(),true,3).unwrap();
 let r=l.run_mission("task-0").unwrap();assert!(r.passed,"{r:?}");
 assert_eq!(r.cost_micros,2634,"two 700-micro model calls plus one 1234-micro checker");
 drop(l);
 let kernel=hs_kernel::Kernel::load(&config).unwrap();
 let resumed=InnerLoop::with_stream(kernel,log.path(),r.stream_id,true,3).unwrap();
 assert_eq!(resumed.total_cost_micros(),2634);
}
