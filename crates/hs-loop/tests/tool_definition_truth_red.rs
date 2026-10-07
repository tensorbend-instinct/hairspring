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
