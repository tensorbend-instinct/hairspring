import {writeFileSync,readdirSync} from 'node:fs';
let current="";const out=[];const sections=[];const failures=[];
const noop=()=>{};
function context() {
 const service=new Proxy(noop,{get:(_,k)=>k==='escalationModes'?[]:k==='promoteOnTimeout'?true:k==='sandboxMode'?undefined:k==='capabilities'?{}:service});
 const ctx=new Proxy({}, {get:(_,k)=>k==='tools'?{register:t=>{out.push({package:current,name:t.name,description:t.description,parameters:t.parameters,output:t.output?.schema,timeoutMs:t.timeoutMs})},get:()=>undefined}:k==='systemPrompt'?{section:s=>sections.push({name:s.name,text:typeof s.text==='string'?s.text:'dynamic'}),getSectionOrder:()=>0}:k==='inject'?(_deps,f)=>f(ctx):k==='get'?()=>undefined:k==='subagents'?{resolveMaxDepth:()=>undefined,getProvider:()=>undefined}:k==='on'?noop:k==='effect'?f=>{try{f()}catch{}}:service});return ctx;
}
for(const n of readdirSync('node_modules/@deepseek-ai').filter(x=>x.startsWith('dsh-tool-'))){
 try {current=n; const m=await import('@deepseek-ai/'+n); const seed=n==='dsh-tool-fs-search'?{sampleOverCapGlobResults:false}:n==='dsh-tool-todo'?{allowParallelInProgress:true}:n==='dsh-tool-subagent'?{provider:'spawn',toolName:'subagent',backgroundMode:'continuable'}:{}; const c=m.Config?m.Config(seed):seed;const start=out.length;m.apply?.(context(),c);console.log(n,out.length-start)} catch(e){failures.push({package:n,error:String(e)});console.log(n,'FAILED',String(e).slice(0,160))}
}
writeFileSync('/tmp/hs/artifacts/tool-diff/dsh-definitions.json',JSON.stringify({out,sections,failures},null,2));
