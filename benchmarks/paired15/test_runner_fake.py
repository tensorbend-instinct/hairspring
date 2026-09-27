"""Entire sequential runner through fake metered DeepSeek, never a real payment."""
import json,os,pathlib,socket,subprocess,tempfile,threading,time,signal,re
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
R=pathlib.Path(__file__).resolve().parent
class Fake(BaseHTTPRequestHandler):
 calls=[];hs_step=0
 def log_message(self,*a):pass
 def do_POST(self):
  v=json.loads(self.rfile.read(int(self.headers['Content-Length'])));self.calls.append({'model':v.get('model'),'stream':v.get('stream'),'tools':[t['function']['name'] for t in v.get('tools',[])]})
  assert self.headers['Authorization']=='Bearer dummy-key-only'
  if v.get('stream'):
   Fake.hs_step+=1
   name='term__exec' if Fake.hs_step==1 else ('answer__submit' if Fake.hs_step==2 else 'verdict__submit')
   prompt=' '.join(m.get('content','') or '' for m in v.get('messages',[]) if isinstance(m.get('content'),str))
   match=re.search(r'ANSWER_PATH: ([^\s]+)',prompt)
   answer_path=match.group(1) if match else '/runstate/mission/work/fake/answer.txt'
   args=({'command':'mkdir -p .hs && printf "true\\n" > .hs/checks && git status --short'} if name=='term__exec' else ({'path':answer_path,'summary':'Offline test submission','checks':'true'} if name=='answer__submit' else {'refuted':False,'blocking':'none','findings':[]}))
   tool={'index':0,'id':'fake_tool_'+str(Fake.hs_step),'type':'function','function':{'name':name,'arguments':json.dumps(args)}}
   chunks=[{'id':'fake','object':'chat.completion.chunk','choices':[{'index':0,'delta':{'role':'assistant','tool_calls':[tool]},'finish_reason':None}]},{'id':'fake','object':'chat.completion.chunk','choices':[{'index':0,'delta':{},'finish_reason':'tool_calls'}]},{'id':'fake','object':'chat.completion.chunk','choices':[],'usage':{'prompt_tokens':10,'completion_tokens':5}}]
   body=b''.join(b'data: '+json.dumps(x).encode()+b'\n\n' for x in chunks)+b'data: [DONE]\n\n';ct='text/event-stream'
  else:
   name='FinishTool' if any(t['function']['name']=='FinishTool' for t in v.get('tools',[])) else ('finish' if any(t['function']['name']=='finish' for t in v.get('tools',[])) else None)
   msg={'role':'assistant','content':'Offline smoke test complete'} if not name else {'role':'assistant','content':None,'tool_calls':[{'id':'fake_tool_1','type':'function','function':{'name':name,'arguments':'{"message":"Offline smoke test complete"}'}}]}
   body=json.dumps({'id':'fake','object':'chat.completion','model':'deepseek-flash','choices':[{'index':0,'message':msg,'finish_reason':'tool_calls' if name else 'stop'}],'usage':{'prompt_tokens':10,'completion_tokens':5}}).encode();ct='application/json'
  self.send_response(200);self.send_header('Content-Length',str(len(body)));self.send_header('Content-Type',ct);self.end_headers();self.wfile.write(body)
def port():
 s=socket.socket();s.bind(('127.0.0.1',0));p=s.getsockname()[1];s.close();return p
with tempfile.TemporaryDirectory(prefix='hs-runner-fake-') as d:
 t=pathlib.Path(d);(t/'bridge').mkdir();key=t/'key';key.write_text('dummy-key-only');key.chmod(0o600)
 upstream=ThreadingHTTPServer(('127.0.0.1',0),Fake);threading.Thread(target=upstream.serve_forever,daemon=True).start()
 proxy=subprocess.Popen(['python3',str(R/'spend_proxy.py'),'--key-file',str(key),'--ledger',str(t/'ledger'),'--port',str(port()),'--upstream',f'http://127.0.0.1:{upstream.server_port}','--ceiling-micros','150000000'],env={**os.environ,'HS_PROXY_TEST_UPSTREAM':'1'},stdout=subprocess.PIPE,stderr=subprocess.PIPE)
 pp=json.loads(proxy.stdout.readline())['port'];bridge=subprocess.Popen(['socat',f'UNIX-LISTEN:{t}/bridge/proxy.sock,fork',f'TCP:127.0.0.1:{pp}'],stderr=subprocess.DEVNULL)
 try:
  for _ in range(100):
   if (t/'bridge/proxy.sock').exists():break
   time.sleep(.02)
  for harness in ['HAIRSPRING','OpenHands']:
   command=['python3',str(R/'runner.py'),'--out',str(t/'runs'),'--id','django__django-11133','--harness',harness,'--port',str(pp),'--ledger',str(t/'ledger'),'--bridge',str(t/'bridge'),'--fake','--grade-smoke']
   child=subprocess.Popen(command,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,start_new_session=True)
   try:out,err=child.communicate(timeout=45)
   except subprocess.TimeoutExpired:
    os.killpg(child.pid,signal.SIGKILL);child.communicate();raise
   run=subprocess.CompletedProcess(command,child.returncode,out,err)
   print(harness,'runner rc',run.returncode,'stdout',run.stdout[-600:],'stderr',run.stderr[-1500:]);assert run.returncode==0
   record=json.loads((t/'runs/django__django-11133'/harness/'record.json').read_text());print(harness,'record', {k:record.get(k) for k in ['state','returncode','steps','actions','outcome','harness_error','wall_secs','patch_sha256']})
   if record['state']!='graded':print(harness,'inner stderr',(t/'runs/django__django-11133'/harness/'stderr.log').read_text()[-2500:]);print(harness,'traffic',Fake.calls[-2:])
   assert record['state']=='graded' and record['resolved'] is False and record['ledger_after']['reserved_micros']==0
   assert record['score']['not_run']==0 and record['score']['fail_to_pass_ok']==0
   assert record['patch_sha256']=='e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855'
  assert len(Fake.calls)==4 and all(x['model']=='deepseek-flash' for x in Fake.calls)
  print('fake model traffic',Fake.calls,'final ledger',json.loads((t/'ledger').read_text()))
 finally:bridge.terminate();proxy.terminate();upstream.shutdown();bridge.wait(timeout=3);proxy.wait(timeout=3)
