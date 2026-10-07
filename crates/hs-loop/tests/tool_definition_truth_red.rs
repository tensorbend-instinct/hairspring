//! The model must receive the actual contracts, not a benchmark fiction.
fn desc(name: &str) -> String {
 hs_loop::toolschema::tb_tools().into_iter().find(|t| t["function"]["name"]==name).unwrap()["function"]["description"].as_str().unwrap().into()
}
#[test]
fn shell_contract_is_truthful_and_routes_background_work_to_jobs() {
 let p=desc("term.exec");
 assert!(!p.contains("you are root"));
 assert!(p.contains("fresh shell"));
 assert!(p.contains("shell variables and cd do not persist"));
 assert!(p.contains("Use jobs"));
 assert!(p.contains("policy denial"));
}
#[test]
fn submit_explains_that_summary_is_the_final_artifact() {
 let p=desc("answer.submit");
 assert!(p.contains("overwrites ANSWER_PATH"));
 assert!(p.contains("summary IS the final answer"));
 assert!(p.contains("one sentence"));
 assert!(p.contains("Do not append verification commentary"));
}

#[test]
fn registry_delegation_keeps_authored_parameters() {
 for (name,required) in [("agent.spawn","mission"),("agent.fork","mission"),("agent.send","child"),("agent.interrupt","child"),("agent.spawn_poll","child_stream_id")]{
  let s=hs_loop::toolschema::schemas_for_registry(&[name.to_string()],&[],"applypatch");
  assert!(s[0]["function"]["parameters"]["properties"][required].is_object(),"{name} lost {required}: {s:?}");
  assert!(!s[0]["function"]["description"].as_str().unwrap().contains("Registered plugin"));
 }
}
