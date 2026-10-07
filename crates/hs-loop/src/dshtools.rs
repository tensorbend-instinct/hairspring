//! dsh-compatible tool contracts and cores. Workspace confinement remains enforced.
use serde_json::{json,Value};
use std::path::{Path,PathBuf};
pub fn schema(name:&str)->Option<Value>{let v:Value=serde_json::from_str(include_str!("dsh_wire_tools.json")).ok()?;v["tools"].as_array()?.iter().find(|t|t["name"]==name).cloned()}
fn validate(args:&Value,allowed:&[&str])->Result<(),String>{let o=args.as_object().ok_or("arguments must be an object")?;for k in o.keys(){if !allowed.contains(&k.as_str()){return Err(format!("unknown argument: {k}"));}}Ok(())}
fn number(args:&Value,key:&str,default:u64,max:u64)->Result<u64,String>{if args.get(key).is_none(){return Ok(default)}let n=args[key].as_u64().ok_or_else(||format!("{key} must be a positive integer"))?;if n==0 || n>max{return Err(format!("{key} must be in 1..={max}"))}Ok(n)}
fn existing(root:&Path,path:&str)->Result<PathBuf,String>{let root=root.canonicalize().map_err(|e|e.to_string())?;let p=if Path::new(path).is_absolute(){PathBuf::from(path)}else{root.join(path)};let p=p.canonicalize().map_err(|e|e.to_string())?;if !p.starts_with(&root){return Err("sandbox: file access denied outside workspace".into())}Ok(p)}
pub fn read(root:&Path,args:&Value)->Result<Value,String>{
 validate(args,&["file_path","offset","limit"])?;
 let path=args["file_path"].as_str().filter(|s|!s.trim().is_empty()).ok_or("file_path must be a non-empty string")?;
 let offset=number(args,"offset",1,u64::MAX)?;let limit=number(args,"limit",2000,2000)?;
 let p=existing(root,path)?;
 let mut lines=vec![];let mut total=0u64;let mut bytes=0usize;let mut capped=false;
 let mut line=String::new();let mut units=0usize;let mut line_present=false;
 let mut consume=|raw:&str|{
  total+=1;if total<offset || lines.len()>=limit as usize || capped{return}
  let raw=raw.strip_suffix('\r').unwrap_or(raw);let mut units=0;let mut cut=raw.len();
  for(i,c)in raw.char_indices(){units+=c.len_utf16();if units>2000{cut=i;break}}
  let line=if cut<raw.len(){format!("{}... (line truncated to 2000 chars)",&raw[..cut])}else{raw.to_string()};
  let n=line.len()+usize::from(!lines.is_empty());if bytes+n>50*1024{capped=true;return}bytes+=n;lines.push(json!({"number":total,"text":line}));
 };
 use std::io::Read;
 let mut file=std::fs::File::open(&p).map_err(|e|e.to_string())?;
 let mut buf=[0u8;8192];let mut carry=Vec::new();
 loop{
  let n=file.read(&mut buf).map_err(|e|e.to_string())?;
  carry.extend_from_slice(&buf[..n]);
  let valid=match std::str::from_utf8(&carry){Ok(_)=>carry.len(),Err(e) if e.error_len().is_none() && n>0=>e.valid_up_to(),Err(_)=>return Err("file is not UTF-8".into())};
  for c in std::str::from_utf8(&carry[..valid]).unwrap().chars(){
   if c=='\n'{consume(&line);line.clear();units=0;line_present=false;}else{line_present=true;if units<2001{line.push(c);units+=c.len_utf16();}}
  }
  carry.drain(..valid);
  if n==0{break}
 }
 if line_present{consume(&line)}
 drop(consume);
 if offset>total && offset!=1{return Err(format!("offset {offset} is beyond end of file ({total} lines)"))}
 let end=lines.last().and_then(|l|l["number"].as_u64()).unwrap_or(offset-1);
 let footer=if capped{format!("(Output capped. Showing lines {offset}-{end}. Use offset={} to continue.)",end+1)}else if end<total{format!("(Showing lines {offset}-{end} of {total}. Use offset={} to continue.)",end+1)}else{format!("(End of file - total {total} lines)")};
 let numbered=lines.iter().map(|l|format!("{}: {}",l["number"],l["text"].as_str().unwrap())).collect::<Vec<_>>().join("\n");let content=if lines.is_empty(){footer}else{format!("{numbered}\n\n{footer}")};let rendered=format!("<path>{}</path>\n<type>file</type>\n<content>\n{content}\n</content>",p.display());
 Ok(json!({"path":p,"offset":offset,"lines":lines,"total_lines":total,"truncated_by_bytes":capped,"text":rendered}))
}
use std::collections::HashMap;
use sha2::{Digest,Sha256};
static MUTATIONS:std::sync::Mutex<()>=std::sync::Mutex::new(());
pub struct FileSession{root:PathBuf,observed:HashMap<PathBuf,String>}
fn version(path:&Path)->Result<String,String>{
 use std::io::Read;let mut f=std::fs::File::open(path).map_err(|e|e.to_string())?;let mut hash=Sha256::new();let mut buf=[0u8;8192];loop{let n=f.read(&mut buf).map_err(|e|e.to_string())?;if n==0{break}hash.update(&buf[..n]);}let m=f.metadata().map_err(|e|e.to_string())?;Ok(format!("{:?}:{:?}:{:x}",m.len(),m.modified(),hash.finalize()))
}
fn target(root:&Path,path:&str)->Result<PathBuf,String>{
 let root=root.canonicalize().map_err(|e|e.to_string())?;let p=if Path::new(path).is_absolute(){PathBuf::from(path)}else{root.join(path)};
 if p.exists(){return existing(&root,path)}
 let mut clean=root.clone();
 let relative=p.strip_prefix(&root).map_err(|_|"sandbox: file access denied outside workspace")?;
 for c in relative.components(){match c{std::path::Component::Normal(n)=>clean.push(n),std::path::Component::CurDir=>{},_=>return Err("sandbox: parent traversal denied".into())}}
 let parent=clean.parent().ok_or("missing parent")?;
 let mut current=root.clone();
 for c in parent.strip_prefix(&root).map_err(|_|"sandbox: outside workspace")?.components(){
  current.push(c.as_os_str());
  match std::fs::symlink_metadata(&current){
   Ok(_)=>{let resolved=current.canonicalize().map_err(|e|e.to_string())?;if !resolved.starts_with(&root){return Err("sandbox: parent symlink escapes workspace".into())}current=resolved;}
   Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{std::fs::create_dir(&current).map_err(|e|e.to_string())?;}
   Err(e)=>return Err(e.to_string())
  }
 }
 let p=current.join(clean.file_name().ok_or("missing name")?);
 if std::fs::symlink_metadata(&p).is_ok(){return Err("sandbox: unresolved symlink target".into())}Ok(p)
}
fn mutation_args(args:&Value,edit:bool)->Result<&str,String>{
 validate(args,if edit{&["file_path","old_string","new_string","replace_all","sandbox_permissions","justification"]}else{&["file_path","content","sandbox_permissions","justification"]})?;
 if args.get("sandbox_permissions").is_some() || args.get("justification").is_some(){return Err("sandbox escalation requires separate user approval; not executed".into())}
 args["file_path"].as_str().filter(|s|!s.trim().is_empty()).ok_or("file_path must be a non-empty string".into())
}
fn atomic(path:&Path,content:&str,create:bool)->Result<(),String>{
 use std::io::Write;
 let parent=path.parent().ok_or("missing parent")?;let stage=parent.join(format!(".hs-{}.tmp",uuid::Uuid::new_v4()));
 let result=(||{
  let mut options=std::fs::OpenOptions::new();options.write(true).create_new(true);
  #[cfg(unix)]{use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
  let mut f=options.open(&stage).map_err(|e|e.to_string())?;f.write_all(content.as_bytes()).map_err(|e|e.to_string())?;f.sync_all().map_err(|e|e.to_string())?;
  if !create{f.set_permissions(std::fs::metadata(path).map_err(|e|e.to_string())?.permissions()).map_err(|e|e.to_string())?;}
  drop(f);
  if create{std::fs::hard_link(&stage,path).map_err(|e|format!("create-if-absent: {e}"))?;}else{std::fs::rename(&stage,path).map_err(|e|e.to_string())?;}
  Ok(())
 })();let _=std::fs::remove_file(stage);result
}
impl FileSession{
 pub fn new(root:&Path)->Self{Self{root:root.to_path_buf(),observed:HashMap::new()}}
 pub fn read(&mut self,args:&Value)->Result<Value,String>{let _lock=MUTATIONS.lock().map_err(|_|"mutation lock poisoned")?;let path=args["file_path"].as_str().ok_or("file_path required")?;let p=existing(&self.root,path)?;let before=version(&p)?;let v=read(&self.root,args)?;let after=version(&p)?;if before!=after{return Err("FS_STALE_VERSION: file changed during read".into())}self.observed.insert(p,after);Ok(v)}
 pub fn write(&mut self,args:&Value)->Result<Value,String>{
  let path=mutation_args(args,false)?;let content=args["content"].as_str().ok_or("content must be a string")?;let _lock=MUTATIONS.lock().map_err(|_|"mutation lock poisoned")?;let p=target(&self.root,path)?;let create=!p.exists();
  if !create{let expected=self.observed.get(&p).ok_or("FS_NOT_OBSERVED: read existing file before overwrite")?;if &version(&p)?!=expected{return Err("FS_STALE_VERSION: file changed since read".into())}}
  let before=if create{Value::Null}else{std::fs::read_to_string(&p).map(|s|json!(s.replace("\r\n","\n"))).unwrap_or(Value::Null)};
  atomic(&p,content,create)?;self.observed.insert(p.clone(),version(&p)?);let word=if create{"Created"}else{"Updated"};Ok(json!({"path":p,"operation":if create{"create"}else{"update"},"before":before,"after":content.replace("\r\n","\n"),"text":format!("<path>{}</path>\n<type>file</type>\n<content>\n{word} file\n</content>",p.display())}))
 }
 pub fn edit(&mut self,args:&Value)->Result<Value,String>{
  let path=mutation_args(args,true)?;let old=args["old_string"].as_str().filter(|s|!s.is_empty()).ok_or("old_string must be a non-empty string")?;let new=args["new_string"].as_str().ok_or("new_string must be a string")?;if old==new{return Err("old_string and new_string must differ".into())}let all=match args.get("replace_all"){None=>false,Some(v)=>v.as_bool().ok_or("replace_all must be boolean")?};
  let _lock=MUTATIONS.lock().map_err(|_|"mutation lock poisoned")?;let p=existing(&self.root,path)?;let expected=self.observed.get(&p).ok_or("FS_NOT_OBSERVED: read file before edit")?;if &version(&p)?!=expected{return Err("FS_STALE_VERSION: file changed since read".into())}
  let original=std::fs::read_to_string(&p).map_err(|e|e.to_string())?;let crlf=original.contains("\r\n");let content=original.replace("\r\n","\n");let old=old.replace("\r\n","\n");let new=new.replace("\r\n","\n");let count=content.matches(&old).count();if count==0{return Err("FS_EDIT_NOT_FOUND: old_string not found".into())}if count>1 && !all{return Err(format!("FS_AMBIGUOUS_EDIT: old_string matched {count} times"))}
  let mut after=content.replace(&old,&new);if crlf{after=after.replace('\n',"\r\n")};atomic(&p,&after,false)?;self.observed.insert(p.clone(),version(&p)?);let message=if all{format!("The file {} has been updated. All occurrences were successfully replaced.",p.display())}else{format!("The file {} has been updated successfully.",p.display())};Ok(json!({"path":p,"before":content,"after":after.replace("\r\n","\n"),"text":message}))
 }
}
