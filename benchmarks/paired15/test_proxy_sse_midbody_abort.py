"""An SSE server that closes mid-body fails closed without a blind retry."""
import http.client,json,os,pathlib,socket,subprocess,sys,tempfile,threading,time,urllib.request
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
R=pathlib.Path(__file__).resolve().parent
class Up(BaseHTTPRequestHandler):
 done=threading.Event();release=threading.Event()
 def log_message(self,*a):pass
 def do_POST(self):
  self.rfile.read(int(self.headers['Content-Length']))
  self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Transfer-Encoding','chunked');self.end_headers()
  payload=b'data: {"choices":[{"delta":{"content":"partial"}}]}\n\n';self.wfile.write(('%X\r\n'%len(payload)).encode()+payload+b'\r\n');self.wfile.flush();Up.done.set();self.close_connection=True
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
   with urllib.request.urlopen(req,timeout=2) as response:
    try:data=response.read()
    except http.client.IncompleteRead as e:data=e.partial
   elapsed=time.monotonic()-start;assert Up.done.is_set(),(elapsed,Up.done.is_set())
   for _ in range(100):
    q=json.loads((t/'ledger').read_text())
    if q['spent_micros']==771860 and q['reserved_micros']==0:break
    time.sleep(.01)
   assert q['spent_micros']==771860 and q['reserved_micros']==0 and not q['blocked'],(elapsed,q)
   assert b'[DONE]' not in data and elapsed<1,(elapsed,data)
   for _ in range(100):
    transitions=[json.loads(x) for x in (t/'proxy.log').read_text().splitlines() if 'proxy_transition' in x]
    if any(x.get('stage')=='settled_at_reserve' for x in transitions):break
    time.sleep(.01)
   assert any(x.get('stage')=='settled_at_reserve' for x in transitions),transitions
   assert sum(x['stage']=='admitted' for x in transitions)==1,transitions
   assert any(x['stage']=='upstream_first_byte' and x.get('upstream_bytes',0)>0 for x in transitions),transitions
   assert any(x['stage']=='upstream_failure_detail' and x.get('upstream_bytes',-1)>0 and x.get('error')=='RemoteProtocolError' for x in transitions),transitions
   print('mid-body SSE close booked reserve once; no hidden retry',round(elapsed,3))
  finally:Up.release.set();proxy.terminate();proxy.wait(timeout=3);up.shutdown()
