#!/usr/bin/env python3
"""Sequential, resumable paired runner. Private hidden tests never enter either agent mount."""
import argparse,datetime,hashlib,json,os,pathlib,shutil,socket,subprocess,sys,time
R=pathlib.Path(__file__).resolve().parent
P=pathlib.Path('/home/sandbox/.local/share/uv/python/cpython-3.12.14-linux-x86_64-gnu')
V=R/'openhands-env12'; BIN=pathlib.Path(__file__).resolve().parents[2]/'target/debug'
GRADE={
 'django__django-15103':str(R/'django41-env/bin/python'),
 'django__django-16877':str(R/'django50-env/bin/python'),
 'django__django-14608':str(R/'django40-env/bin/python'),
 'django__django-13343':str(R/'django32-env/bin/python'),
 'django__django-13590':str(R/'django32-env/bin/python'),
 'django__django-13821':str(R/'django32-env/bin/python'),
 'django__django-11141':str(R/'django31-env/bin/python'),
 'django__django-11133':str(R/'django38-env/bin/python'),
 'django__django-11490':str(R/'django38-env/bin/python'),
 'pylint-dev__':str(R/'pylint38-env/bin/python'),
 'pytest-dev__':str(R/'pytest38-env/bin/python'),
 'scikit-learn__':str(R/'sklearn38-env/bin/python'),
 'sphinx-doc__':str(R/'sphinx38-env/bin/python'),
 'sympy__':str(R/'sympy310-env/bin/python'),
}
OH_SCRIPT='''
import json,pathlib,socket,subprocess,time,traceback
from openhands.sdk import LLM,Agent,Conversation
from openhands.sdk.tool.spec import Tool
from openhands.tools import register_default_tools,TerminalTool,FileEditorTool
register_default_tools(enable_browser=False)
from openhands.sdk.workspace import LocalWorkspace
from openhands.sdk.event import ActionEvent,AgentErrorEvent,MessageEvent
from openhands.sdk.event.conversation_error import ConversationErrorEvent
assert not pathlib.Path('/tmp/hs-audit').exists()
assert not pathlib.Path('/runstate/test.patch').exists()
s=socket.socket();assert s.connect_ex(('1.1.1.1',443))!=0
relay=subprocess.Popen(['socat','TCP-LISTEN:18748,bind=127.0.0.1,reuseaddr,fork','UNIX-CONNECT:/bridge/proxy.sock'],stderr=subprocess.DEVNULL)
try:
 for _ in range(100):
  try:
   with socket.create_connection(('127.0.0.1',18748),timeout=.1):break
  except OSError:time.sleep(.02)
 else:raise RuntimeError('proxy socket did not accept connections')
 llm=LLM(model='deepseek/deepseek-flash',base_url='http://127.0.0.1:18748/v1',api_key='local-placeholder',max_output_tokens=393216,usage_id='benchmark-agent',stream=False,num_retries=0)
 agent=Agent(llm=llm,tools=[Tool(name=TerminalTool.name),Tool(name=FileEditorTool.name)])
 conv=Conversation(agent=agent,workspace=LocalWorkspace(working_dir=pathlib.Path('/workspace')),persistence_dir='/persistence',max_iteration_per_run=1 if pathlib.Path('/runstate/smoke').exists() else 2147483647,stuck_detection=False,visualizer=None,delete_on_close=False)
 try:
  conv.send_message(pathlib.Path('/runstate/problem.txt').read_text())
  conv.run()
  events=list(conv.state.events)
  summary={'harness':'OpenHands','status':str(conv.state.execution_status),'steps':len({e.llm_response_id for e in events if isinstance(e,(ActionEvent,MessageEvent)) and e.llm_response_id}),'actions':sum(isinstance(e,ActionEvent) for e in events),'errors':sum(isinstance(e,(AgentErrorEvent,ConversationErrorEvent)) for e in events),'events':len(events),'model':'deepseek/deepseek-flash'}
  pathlib.Path('/runstate/summary.json').write_text(json.dumps(summary,indent=2)+'\\n')
  print(json.dumps(summary),flush=True)
 finally:conv.close()
finally:relay.terminate();relay.wait(timeout=5)
'''
HS_SCRIPT='''
import pathlib,subprocess,socket,time,os
assert not pathlib.Path('/home/sandbox/recovery').exists()
assert not pathlib.Path('/runstate/test.patch').exists()
s=socket.socket();assert s.connect_ex(('1.1.1.1',443))!=0
relay=subprocess.Popen(['socat','TCP-LISTEN:18748,bind=127.0.0.1,reuseaddr,fork','UNIX-CONNECT:/bridge/proxy.sock'],stderr=subprocess.DEVNULL)
try:
 for _ in range(100):
  try:
   with socket.create_connection(('127.0.0.1',18748),timeout=.1):break
  except OSError:time.sleep(.02)
 else:raise RuntimeError('proxy socket did not accept connections')
 env={**os.environ,'PATH':'/opt/hs:/usr/bin:/bin','HOME':'/workspace','HS_DEEPSEEK_API_KEY':'local-placeholder','HS_DEEPSEEK_BASE_URL':'http://127.0.0.1:18748/chat/completions','HS_DEEPSEEK_MODEL':'deepseek-flash','HS_CRITIC_MODEL':'deepseek','HS_REALMODEL_CALL_TIMEOUT_SECS':'960','HS_POLICY_TOML':'/runstate/policy.toml','HS_TUI':'off'}
 if pathlib.Path('/runstate/smoke').exists():env['HS_CRITIC_SCRIPT']='tool:git status --short|clean'
 cmd=['/opt/hs/hs-repl','run','--goal',pathlib.Path('/runstate/problem.txt').read_text(),'--config','/runstate/rig.toml','--dir','/runstate/mission','--project-dir','/workspace']
 if pathlib.Path('/runstate/smoke').exists():cmd+=['--max-steps','3']
 raise SystemExit(subprocess.run(cmd,env=env).returncode)
finally:relay.terminate();relay.wait(timeout=5)
'''
def atom(p,v):
 t=p.with_suffix('.tmp');t.write_text(json.dumps(v,indent=2)+'\n');t.replace(p)
def ledger(p):
 try:return json.loads(p.read_text())
 except Exception:return {'error':'ledger unreadable'}
def preflight_proxy(port):
 with socket.create_connection(('127.0.0.1',port),timeout=3):pass
# Case preparation reuses only the pristine audited original; a new clone each side.
def prepare(case,harness,out):
 src=case/'source';work=out/'workspace';subprocess.run(['git','clone','--quiet','--no-hardlinks',str(src),str(work)],check=True)
 base=json.loads((case/'public.json').read_text())['base_commit'];subprocess.run(['git','checkout','--quiet',base],cwd=work,check=True)
 assert subprocess.check_output(['git','rev-parse','HEAD'],cwd=work,text=True).strip()==base
 assert not subprocess.check_output(['git','status','--porcelain'],cwd=work,text=True)
 assert not (work/'test.patch').exists() and not (work/'private.json').exists()
 assert not (src/'test.patch').exists() and not (src/'private.json').exists()
 assert not subprocess.check_output(['git','status','--porcelain'],cwd=src,text=True)
 return work
def grade_python(id):
 for prefix,path in GRADE.items():
  if id.startswith(prefix):return path
 return str(R/'django38-env/bin/python')
def run_one(id,harness,out,port,ledger_path,bridge,fake=False,grade_smoke=False):
 candidates=list(R.glob('probe-*'))
 case=next((c for c in candidates if (c/'public.json').exists() and json.loads((c/'public.json').read_text())['instance_id']==id),None)
 if case is None:raise RuntimeError('missing audited preflight case '+id)
 public=json.loads((case/'public.json').read_text());private=json.loads((case/'private.json').read_text())
 if public['instance_id']!=id or private['instance_id']!=id:raise RuntimeError('preflight identity mismatch')
 if hashlib.sha256((case/'test.patch').read_bytes()).hexdigest()!=private['test_patch_sha256']:raise RuntimeError('hidden test digest mismatch')
 if not fake and not pathlib.Path(grade_python(id)).is_file():raise RuntimeError('grading Python missing for '+id)
 if out.exists():raise RuntimeError('run directory already exists, do not overwrite: '+str(out))
 out.mkdir(parents=True);work=prepare(case,harness,out);state=out/'state';state.mkdir();(out/'persistence').mkdir()
 (state/'problem.txt').write_text(json.loads((case/'public.json').read_text())['problem_statement']+'\n\nExecution constraints: Network access is unavailable inside this sandbox. Work with the checked-out repository and installed local tools only. Do not modify, add, or delete test files; only change implementation files. Hidden grading happens after you finish.\n')
 if harness=='HAIRSPRING':
  (state/'policy.toml').write_text('[prompts]\ntui-mission = """{goal}\n\nExecution constraints: Network access is unavailable inside this sandbox. Work with the checked-out repository and installed local tools only. Do not modify, add, or delete test files; only change implementation files. Hidden grading happens after you finish.\n"""\n')
 if grade_smoke:(state/'smoke').write_text('test-only bounded fake model')
 if harness=='HAIRSPRING':(state/'rig.toml').write_text((R/'rig.template.toml').read_text().replace('__BIN_DIR__','/opt/hs'))
 status={'instance_id':id,'harness':harness,'state':'started','started_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'ledger_before':ledger(ledger_path)};atom(out/'record.json',status)
 binds=['bwrap','--unshare-net','--ro-bind','/usr','/usr','--ro-bind','/bin','/bin','--ro-bind','/lib','/lib','--ro-bind','/lib64','/lib64','--ro-bind','/etc','/etc','--proc','/proc','--dev','/dev','--tmpfs','/tmp','--dir','/workspace','--bind',str(work),'/workspace','--dir','/runstate','--bind',str(state),'/runstate','--dir','/bridge','--bind',str(bridge),'/bridge','--chdir','/workspace']
 if harness=='HAIRSPRING':
  cmd=binds+['--ro-bind',str(BIN),'/opt/hs','--setenv','PATH','/opt/hs:/usr/bin:/bin','--','python3','-u','-c',HS_SCRIPT]
 else:
  cmd=binds+['--ro-bind',str(P),'/opt/python','--ro-bind',str(V),'/opt/venv','--dir','/persistence','--bind',str(out/'persistence'),'/persistence','--setenv','PATH','/opt/venv/bin:/opt/python/bin:/usr/bin:/bin','--setenv','PYTHONPATH','/opt/venv/lib/python3.12/site-packages','--setenv','VIRTUAL_ENV','/opt/venv','--setenv','LITELLM_LOCAL_MODEL_COST_MAP','true','--setenv','OPENHANDS_SUPPRESS_BANNER','1','--','/opt/python/bin/python3.12','-u','-c',OH_SCRIPT]
 t=time.monotonic()
 with (out/'stdout.log').open('w') as so,(out/'stderr.log').open('w') as se:
  p=subprocess.Popen(cmd,stdout=so,stderr=se);status['pid']=p.pid;atom(out/'record.json',status)
  try:rc=p.wait()
  except KeyboardInterrupt:
   status['state']='interrupted';atom(out/'record.json',status);raise
 status.update({'state':'agent_finished','returncode':rc,'wall_secs':round(time.monotonic()-t,3),'ledger_after':ledger(ledger_path)})
 # The patch is always stored before any grader attempt, including a failed mission.
 patch=subprocess.check_output(['git','diff','--binary','HEAD','--'],cwd=work);(out/'candidate.patch').write_bytes(patch)
 status['patch_sha256']=hashlib.sha256(patch).hexdigest()
 status['untracked']=subprocess.check_output(['git','ls-files','--others','--exclude-standard'],cwd=work,text=True).splitlines()
 if harness=='OpenHands' and (state/'summary.json').exists():status.update(json.loads((state/'summary.json').read_text()))
 if harness=='HAIRSPRING':
  lines=(out/'stdout.log').read_text(errors='replace').splitlines();records=[]
  for line in lines:
   try:
    obj=json.loads(line)
    if isinstance(obj,dict) and 'model_calls' in obj and 'steps' in obj:records.append(obj)
   except ValueError:pass
  if records:status.update({k:records[-1].get(k) for k in ('steps','model_calls','harness_error','outcome','passed','answer_path','stream_id')})
 atom(out/'record.json',status)
 if fake and not grade_smoke:return status
 # Any nonterminal/error must count unfinished even if source happens to pass.
 finished=(harness=='OpenHands' and rc==0 and status.get('status') in ('ConversationExecutionStatus.FINISHED','finished') and status.get('errors')==0) or (harness=='HAIRSPRING' and rc==0 and status.get('outcome')=='verified' and status.get('passed') is True and not status.get('harness_error'))
 if not finished:
  status['resolved']=False
  status['state']='unfinished';atom(out/'record.json',status);return status
 score=out/'score';python=grade_python(id)
 with (out/'grade.log').open('w') as log:
  result=subprocess.run([sys.executable,str(R/'score_native.py'),'--case',str(case),'--agent-tree',str(work),'--python',python,'--out',str(score)],stdout=log,stderr=subprocess.STDOUT)
 status['grade_returncode']=result.returncode
 if (score/'score.json').exists():status['score']=json.loads((score/'score.json').read_text())
 status['resolved']=bool(status.get('score',{}).get('resolved') and result.returncode==0)
 status['state']='graded' if 'score' in status else 'grade_failed';atom(out/'record.json',status);return status

def main():
 ap=argparse.ArgumentParser();ap.add_argument('--out',required=True);ap.add_argument('--port',type=int,default=18748);ap.add_argument('--ledger',required=True);ap.add_argument('--bridge',required=True);ap.add_argument('--fake',action='store_true');ap.add_argument('--grade-smoke',action='store_true');ap.add_argument('--id');ap.add_argument('--harness',choices=['HAIRSPRING','OpenHands']);a=ap.parse_args()
 manifest=json.loads((R/'manifest.json').read_text())
 if hashlib.sha256((R/'verified.parquet').read_bytes()).hexdigest()!=manifest['source_sha256']:raise RuntimeError('dataset digest mismatch')
 ids=manifest['ids'];out=pathlib.Path(a.out).resolve();out.mkdir(parents=True,exist_ok=True)
 if a.grade_smoke and not a.fake:ap.error('grade smoke is fake-only')
 if a.id and a.id not in ids:ap.error('not in locked sample')
 if a.id and not a.harness:ap.error('single ID requires harness')
 if a.harness and not a.id:ap.error('harness requires a single ID')
 if not a.fake:
  if not (BIN/'hs-repl').is_file() or not V.is_dir() or not P.is_dir():raise RuntimeError('runtime dependencies absent; do not start paid work')
  preflight_proxy(a.port)
  initial=ledger(pathlib.Path(a.ledger))
  if initial.get('blocked') or initial.get('reserved_micros') or initial.get('error') or initial.get('spent_micros',0)<6052157:raise RuntimeError('recovered ledger missing, unsettled, or below conservative baseline')
 bridge=pathlib.Path(a.bridge).resolve();assert (bridge/'proxy.sock').exists()
 for id in ([a.id] if a.id else ids):
  for h in ([a.harness] if a.harness else ['HAIRSPRING','OpenHands']):
   path=out/id/h
   if (path/'record.json').exists():
    old=json.loads((path/'record.json').read_text())
    if old['state'] in ('graded','unfinished'):
     print('already recorded',id,h,old['state'],flush=True);continue
    raise RuntimeError('prior run unfinished or ambiguous; do not silently rerun '+str(path))
   result=run_one(id,h,path,a.port,pathlib.Path(a.ledger),bridge,a.fake,a.grade_smoke)
   print(json.dumps({'id':id,'harness':h,'state':result['state'],'resolved':result.get('resolved'),'steps':result.get('steps'),'wall_secs':result.get('wall_secs'),'spent_micros':result['ledger_after'].get('spent_micros')}),flush=True)
   if result['state'] not in ('graded','unfinished') and not a.fake:raise RuntimeError('grade failed; stop batch')
   if result['state']=='unfinished' and not a.fake:raise RuntimeError('unfinished task; stop sequential batch for diagnosis')
   if result['ledger_after'].get('blocked') or result['ledger_after'].get('reserved_micros'):raise RuntimeError('ledger blocked or unsettled; stop batch')
if __name__=='__main__':main()
