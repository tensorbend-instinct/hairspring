"""A slow coordinator does not kill the provider call or double-settle."""
import json,os,pathlib,socket,subprocess,sys,tempfile,threading,time,urllib.request
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
R=pathlib.Path(__file__).resolve().parent
class Up(BaseHTTPRequestHandler):
 def log_message(self,*a):pass
 def do_POST(self):
  self.rfile.read(int(self.headers['Content-Length']))
  b=b'data: {"choices":[]}\n\ndata: {"usage":{"prompt_tokens":10,"completion_tokens":5},"choices":[]}\n\ndata: [DONE]\n\n'
  self.send_response(200);self.send_header('Content-Length',str(len(b)));self.send_header('Content-Type','text/event-stream');self.end_headers();self.wfile.write(b)
def port():
 s=socket.socket();s.bind(('127.0.0.1',0));p=s.getsockname()[1];s.close();return p
with tempfile.TemporaryDirectory(prefix='hs-coordinator-deadline-') as d:
 t=pathlib.Path(d);key=t/'key';key.write_text('dummy');key.chmod(0o600)
 up=ThreadingHTTPServer(('127.0.0.1',0),Up);threading.Thread(target=up.serve_forever,daemon=True).start()
 pp=port();env={**os.environ,'HS_PROXY_TEST_UPSTREAM':'1','HS_PROXY_TEST_DEADLINE_SECS':'.25','HS_PROXY_TEST_BLOCK_FORWARD_SECS':'.8','HS_PROXY_TEST_HOLD_COORDINATOR_LOCK':'1'}
 with (t/'proxy.log').open('w') as log:
  proxy=subprocess.Popen([sys.executable,str(R/'spend_proxy.py'),'--key-file',str(key),'--ledger',str(t/'ledger'),'--port',str(pp),'--upstream',f'http://127.0.0.1:{up.server_port}'],env=env,stdout=log,stderr=subprocess.STDOUT)
  try:
   for _ in range(100):
    try:
     with socket.create_connection(('127.0.0.1',pp),timeout=.1):break
    except OSError:time.sleep(.02)
   body=json.dumps({'model':'deepseek-flash','stream':True,'stream_options':{'include_usage':True}}).encode()
   req=urllib.request.Request(f'http://127.0.0.1:{pp}/chat/completions',data=body)
   result={}
   def client():
    try:result['data']=urllib.request.urlopen(req,timeout=4).read()
    except Exception as e:result['error']=repr(e)
   client_thread=threading.Thread(target=client,daemon=True);client_thread.start()
   for _ in range(100):
    if 'coordinator_forward_enter' in (t/'proxy.log').read_text():break
    time.sleep(.01)
   else:raise AssertionError('coordinator did not enter forward path')
   time.sleep(.35)
   q=json.loads((t/'ledger').read_text());assert q['reserved_micros']==771860 and q['spent_micros']==0,q
   client_thread.join(timeout=4);assert not client_thread.is_alive() and 'data' in result,result
   assert b'data: [DONE]' in result['data'],result
   q=json.loads((t/'ledger').read_text());assert q['spent_micros']==9 and q['reserved_micros']==0,q
   transitions=[json.loads(x) for x in (t/'proxy.log').read_text().splitlines() if 'proxy_transition' in x]
   assert len([x for x in transitions if x.get('stage')=='settled_usage'])==1,transitions
   print('blocked coordinator left call alive; final usage settled exactly once')
  finally:proxy.terminate();proxy.wait(timeout=3);up.shutdown()
