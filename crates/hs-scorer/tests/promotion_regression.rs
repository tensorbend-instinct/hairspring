use hs_scorer::{Artifact,Candidate,Lineage,Scorer,ScorerConfig,Task,TaskSuite,Tier01,Tier02,PromotionError};

fn suite() -> TaskSuite { TaskSuite::new("held",vec![Task::new("one".into(),"token".into())]) }
fn candidate(name:&str) -> Candidate { Candidate::new(name, Artifact::by_rule(|t|Some(t.secret().to_owned()))) }
fn scores() -> (Tier01,Tier02) {
 (Tier01 {passed:true,tasks_correct:1,tasks_total:1},
  Tier02 {mean:1.0,ci_low:1.0,ci_high:1.0,veto:false,families:1,veto_weight:0.5,
          cross_family_disagreement:0.0,same_family_disagreement:0.0})
}
#[test]
fn verdict_must_match_recorded_candidate() {
 let tmp=tempfile::tempdir().unwrap();let mut scorer=Scorer::new(&tmp.path().join("log"),ScorerConfig::default()).unwrap();
 let pin=scorer.pin();let a=candidate("a");let b=candidate("b");
 let verdict=scorer.held_out_assay(&a,&suite(),&pin).unwrap();
 let (s0,s1)=scores();let mut line=Lineage::new(tmp.path().join("line"),"family").unwrap();
 line.record(&a,s0,s1,verdict.clone());
 assert!(matches!(line.promote(&b,&verdict,&pin,&scorer),Err(PromotionError::VerdictMismatch)));
 assert!(line.champion().is_none());
 line.promote(&a,&verdict,&pin,&scorer).unwrap();
 assert_eq!(line.champion().unwrap().name(),"a");
}
#[test]
fn failed_record_write_does_not_promote() {
 let tmp=tempfile::tempdir().unwrap();let mut scorer=Scorer::new(&tmp.path().join("log"),ScorerConfig::default()).unwrap();
 let pin=scorer.pin();let a=candidate("a");let verdict=scorer.held_out_assay(&a,&suite(),&pin).unwrap();
 let mut line=Lineage::new(tmp.path().join("line"),"family").unwrap();let (s0,s1)=scores();line.record(&a,s0,s1,verdict.clone());
 std::fs::create_dir(tmp.path().join("line/promotion-a.json")).unwrap();
 assert!(matches!(line.promote(&a,&verdict,&pin,&scorer),Err(PromotionError::Persistence(_))));
 assert!(line.champion().is_none());
}
