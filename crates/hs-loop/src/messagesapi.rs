//! DeepSeek Messages protocol conversion. Stock SSE codec, provider-shaped block assembly.
use serde_json::{json,Value};
use std::collections::BTreeSet;
pub fn from_chat(b:&Value)->Result<Value,String>{
 let mut system=Vec::new();let mut msgs=Vec::new();let mut pending=BTreeSet::new();
 for m in b["messages"].as_array().ok_or("messages array required")? {
  match m["role"].as_str().unwrap_or("") {
   "system"=>system.push(m["content"].as_str().unwrap_or("").to_string()),
   "assistant"=>{
    if let Some(blocks)=m["content"].as_array(){
      for block in blocks {if block["type"]=="tool_use" {let id=block["id"].as_str().ok_or("native tool id missing")?;if !pending.insert(id.to_string()){return Err("duplicate native tool id".into());}}}
      msgs.push(json!({"role":"assistant","content":blocks}));continue;
    }
    let mut cs=Vec::new();
    if let Some(r)=m["reasoning_content"].as_str().filter(|r|!r.is_empty()){cs.push(json!({"type":"thinking","thinking":r}));}
    if let Some(c)=m["content"].as_str().filter(|c|!c.is_empty()){cs.push(json!({"type":"text","text":c}));}
    if let Some(ts)=m["tool_calls"].as_array(){for t in ts {let id=t["id"].as_str().ok_or("tool id required")?;if !pending.insert(id.to_string()){return Err("duplicate tool id".into());}let input:Value=serde_json::from_str(t["function"]["arguments"].as_str().ok_or("tool arguments required")?).map_err(|e|e.to_string())?;if !input.is_object(){return Err("tool input must be object".into());}cs.push(json!({"type":"tool_use","id":id,"name":t["function"]["name"],"input":input}));}}
    if cs.is_empty(){cs.push(json!({"type":"text","text":""}));}msgs.push(json!({"role":"assistant","content":cs}));
   },
   "tool"=>{let id=m["tool_call_id"].as_str().ok_or("tool result id required")?;if !pending.remove(id){return Err("orphan tool result".into());}let content=m["content"].as_str().map(str::to_string).unwrap_or_else(||m["content"].to_string());let result=json!({"type":"tool_result","tool_use_id":id,"content":content});
    if let Some(last)=msgs.last_mut().filter(|m|m["role"]=="user" && m["content"].as_array().is_some_and(|bs|bs.iter().all(|b|b["type"]=="tool_result"))){last["content"].as_array_mut().unwrap().push(result);}else{msgs.push(json!({"role":"user","content":[result]}));}},
   "user"=>{if !pending.is_empty(){return Err("tool results must precede next user message".into());}msgs.push(json!({"role":"user","content":m["content"]}));},
   _=>return Err("unsupported message role".into())
  }
 }
 if !pending.is_empty(){return Err("unresolved tool uses".into());}
 let mut out=json!({"model":b["model"],"messages":msgs,"max_tokens":b["max_tokens"],"system":system.join("\n\n"),"thinking":{"type":"enabled"},"output_config":{"effort":"high"}});
 if let Some(ts)=b["tools"].as_array(){out["tools"]=Value::Array(ts.iter().map(|t|{let f=&t["function"];json!({"name":f["name"],"description":f["description"],"input_schema":f["parameters"]})}).collect());}
 for key in ["temperature","thinking","output_config"]{if !b[key].is_null(){out[key]=b[key].clone();}}
 Ok(out)
}
pub fn to_chat(v:&Value)->Result<Value,String>{
 if v["type"]=="error"{return Err(format!("Messages error: {}",v["error"]["message"]));}
 let mut text=String::new();let mut reasoning=String::new();let mut tools=Vec::new();
 for c in v["content"].as_array().ok_or("Messages content missing")?{match c["type"].as_str(){Some("text")=>text.push_str(c["text"].as_str().ok_or("text missing")?),Some("thinking")=>reasoning.push_str(c["thinking"].as_str().unwrap_or("")),Some("tool_use")=>{if !c["input"].is_object() || !c["name"].is_string(){return Err("malformed tool_use".into());}tools.push(json!({"id":c["id"],"type":"function","function":{"name":c["name"],"arguments":c["input"].to_string()}}));},_=>{}}}
 let u=&v["usage"];let cached=u["cache_read_input_tokens"].as_u64().unwrap_or(0);let total=u["input_tokens"].as_u64().unwrap_or(0)+cached+u["cache_creation_input_tokens"].as_u64().unwrap_or(0);
 Ok(json!({"model":v["model"],"choices":[{"finish_reason":v["stop_reason"],"message":{"role":"assistant","content":text,"reasoning_content":reasoning,"tool_calls":tools,"messages_content":v["content"]}}],"usage":{"prompt_tokens":total,"prompt_cache_hit_tokens":cached,"completion_tokens":u["output_tokens"]}}))
}
pub struct Stream {
 codec:sse_codec::SSECodec, bytes:futures_codec::BytesMut, message:Value, blocks:std::collections::BTreeMap<u64,Value>, partial:std::collections::BTreeMap<u64,String>, done:bool,
}
impl Default for Stream{fn default()->Self{Self{codec:sse_codec::SSECodec::default(),bytes:futures_codec::BytesMut::new(),message:Value::Null,blocks:Default::default(),partial:Default::default(),done:false}}}
impl Stream{
 pub fn is_done(&self)->bool{self.done}
 pub fn push(&mut self,b:&[u8],delta:&mut dyn FnMut(&str))->Result<(),String>{
  use futures_codec::Decoder;self.bytes.extend_from_slice(b);
  while let Some(ev)=self.codec.decode(&mut self.bytes).map_err(|e|e.to_string())?{
   let sse_codec::Event::Message{event,data,..}=ev else{continue};
   if event=="ping"{continue;}let v:Value=serde_json::from_str(&data).map_err(|e|e.to_string())?;
   match v["type"].as_str().unwrap_or(&event){
    "message_start"=>{if !self.message.is_null(){return Err("duplicate message_start".into());}self.message=v["message"].clone();},
    "content_block_start"=>{let i=v["index"].as_u64().ok_or("block index missing")?;if self.blocks.insert(i,v["content_block"].clone()).is_some(){return Err("duplicate block".into());}},
    "content_block_delta"=>{let i=v["index"].as_u64().ok_or("delta index missing")?;let b=self.blocks.get_mut(&i).ok_or("delta before block start")?;let d=&v["delta"];match d["type"].as_str(){
     Some("text_delta")=>{let t=d["text"].as_str().ok_or("text delta missing")?;let prev=b["text"].as_str().unwrap_or("").to_string();b["text"]=json!(prev+t);delta(t);},
     Some("thinking_delta")=>{let t=d["thinking"].as_str().ok_or("thinking delta missing")?;let prev=b["thinking"].as_str().unwrap_or("").to_string();b["thinking"]=json!(prev+t);},
     Some("input_json_delta")=>{let t=d["partial_json"].as_str().ok_or("JSON delta missing")?;self.partial.entry(i).or_default().push_str(t);delta(t);},
     Some("signature_delta")=>b["signature"]=d["signature"].clone(),_=>{}}
    },
    "content_block_stop"=>{},
    "message_delta"=>{self.message["stop_reason"]=v["delta"]["stop_reason"].clone();if let Some(u)=v["usage"].as_object(){for(k,x)in u{self.message["usage"][k]=x.clone();}}},
    "message_stop"=>{if self.message.is_null(){return Err("message_stop without start".into());}self.done=true;},
    "error"=>return Err(format!("Messages stream error: {}",v["error"]["message"])),_=>{}
   }
  }Ok(())
 }
 pub fn finish(mut self)->Result<Value,String>{
  if !self.done{return Err("incomplete Messages stream: no message_stop".into());}
  for(i,s)in self.partial{self.blocks.get_mut(&i).ok_or("missing JSON block")?["input"]=serde_json::from_str(&s).map_err(|e|format!("malformed tool JSON: {e}"))?;}
  self.message["content"]=Value::Array(self.blocks.into_values().collect());
  if !self.message["usage"].is_object(){return Err("Messages usage missing".into());}to_chat(&self.message)
 }
}
pub fn assemble_sse(s:&str)->Result<Value,String>{let mut stream=Stream::default();stream.push(s.as_bytes(),&mut |_|{})?;stream.finish()}
