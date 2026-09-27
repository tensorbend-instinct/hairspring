"""A complete SSE [DONE] plus usage settles even if HTTP connection stays open."""
import http.client,json,os,pathlib,socket,subprocess,sys,tempfile,threading,time,urllib.request
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
R=pathlib.Path(__file__).resolve().parent
class Up(BaseHTTPRequestHandler):
 done=threading.Event();release=threading.Event()
 def log_message(self,*a):pass
 def do_POST(self):
  self.rfile.read(int(self.headers['Content-Length']))
  self.send_response(200);self.send_header('Content-Type','text/event-stream');self.end_headers()
  self.wfile.write(b'data: {"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}}\n\ndata: [DONE]\n\n');self.wfile.flush();Up.done.set();Up.release.wait(2)
def port():
 s=socket.socket();s.bind(('127.0.0.1',0));p=s.getsockname()[1];s.close();return p
with tempfile.TemporaryDirectory() as d:
 t=pathlib.Path(d);key=t/'key';key.write_text('dummy');key.chmod(0o600)
 up=ThreadingHTTPServer(('127.0.0.1',0),Up);threading.Thread(target=up.serve_forever,daemon=True).start()
 pp=port();env={**os.environ,'HS_PROXY_TEST_UPSTREAM':'1','HS_PROXY_TEST_DEADLINE_SECS':'.4'}
 with (t/'proxy.log').open('w') as log:
  proxy=subprocess.Popen([sys.executable,str(R/'spend_proxy.py'),'--key-file',str(key),'--ledger',str(t/'ledger'),'--port',str(pp),'--upstream',f'http://127.0.0.1:{up.server_port}'],env=env,stdout=log,stderr=subprocess.STDOUT)
  try:
   for _ in range(100):
    try:
     with socket.create_connection(('127.0.0.1',pp),timeout=.1):break
    except OSError:time.sleep(.02)
   body=json.dumps({'model':'deepseek-flash','stream':True,'stream_options':{'include_usage':True}}).encode()
   req=urllib.request.Request(f'http://127.0.0.1:{pp}/chat/completions',data=body)
   start=time.monotonic()
   with urllib.request.urlopen(req,timeout=2) as response:data=response.read()
   elapsed=time.monotonic()-start;assert Up.done.is_set() and not Up.release.is_set(),(elapsed,Up.done.is_set(),Up.release.is_set())
   for _ in range(100):
    q=json.loads((t/'ledger').read_text())
    if q['spent_micros']==9 and q['reserved_micros']==0:break
    time.sleep(.01)
   assert q['spent_micros']==9 and q['reserved_micros']==0 and not q['blocked'],(elapsed,q)
   assert b'[DONE]' in data and elapsed<.30,(elapsed,data)
   for _ in range(100):
    transitions=[json.loads(x) for x in (t/'proxy.log').read_text().splitlines() if 'proxy_transition' in x]
    if any(x.get('stage')=='settled_usage' for x in transitions):break
    time.sleep(.01)
   assert any(x.get('stage')=='settled_usage' for x in transitions),transitions
   print('terminal SSE settled with usage while upstream connection remained open',round(elapsed,3))
  finally:Up.release.set();proxy.terminate();proxy.wait(timeout=3);up.shutdown()
