use hs_loop::critic::*;
struct Metered { rounds:u64 }
impl CriticModel for Metered {
 fn step(&mut self,_:&[serde_json::Value])->Result<CriticReply,String> {
  self.rounds+=1;
  Ok(CriticReply::Final("{\"refuted\":false,\"reason\":\"checked\"}".into()))
 }
 fn usage(&self)->(u64,u64,u64){(self.rounds*20,self.rounds*7,self.rounds*900)}
}
#[test]
fn critic_rounds_book_exact_usage_and_end_time() {
 let dir=tempfile::tempdir().unwrap();let mut m=Metered{rounds:0};
 let r=refute(dir.path(),"check","true",&Default::default(),&mut m);
 let end=r.trace.iter().find(|v|v["kind"]=="model_end").unwrap();
 assert_eq!(end["cost_micros"],900);assert_eq!(end["input_tokens"],20);
 assert_eq!(end["output_tokens"],7);assert!(end["ts_wall_ms"].is_number());
 assert_eq!(r.cost_micros,900);
}
