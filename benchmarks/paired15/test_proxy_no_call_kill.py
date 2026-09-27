"""A live call survives the former absolute deadline and settles actual usage."""
import json,os,pathlib,socket,subprocess,sys,tempfile,threading,time,urllib.request
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
R=pathlib.Path(__file__).resolve().parent
class Up(BaseHTTPRequestHandler):
 def log_message(self,*a):pass
 def do_POST(self):
  self.rfile.read(int(self.headers['Content-Length']))
  time.sleep(.5)
  b=json.dumps({'choices':[],'usage':{'prompt_tokens':10,'completion_tokens':5}}).encode()
  self.send_response(200);self.send_header('Content-Length',str(len(b)));self.end_headers();self.wfile.write(b)
def port():
 s=socket.socket();s.bind(('127.0.0.1',0));p=s.getsockname()[1];s.close();return p
with tempfile.TemporaryDirectory() as d:
 t=pathlib.Path(d);key=t/'key';key.write_text('dummy');key.chmod(0o600)
 up=ThreadingHTTPServer(('127.0.0.1',0),Up);threading.Thread(target=up.serve_forever,daemon=True).start()
 pp=port();env={**os.environ,'HS_PROXY_TEST_UPSTREAM':'1','HS_PROXY_TEST_DEADLINE_SECS':'.2'}
 with (t/'proxy.log').open('w') as log:
  p=subprocess.Popen([sys.executable,str(R/'spend_proxy.py'),'--key-file',str(key),'--ledger',str(t/'ledger'),'--port',str(pp),'--upstream',f'http://127.0.0.1:{up.server_port}'],env=env,stdout=log,stderr=subprocess.STDOUT)
  try:
   for _ in range(100):
    try:
     with socket.create_connection(('127.0.0.1',pp),timeout=.1):break
    except OSError:time.sleep(.02)
   body=json.dumps({'model':'deepseek-flash','stream':False}).encode()
   start=time.monotonic()
   with urllib.request.urlopen(urllib.request.Request(f'http://127.0.0.1:{pp}/chat/completions',data=body),timeout=2) as resp:
    assert json.loads(resp.read())['usage']['completion_tokens']==5
   assert time.monotonic()-start>=.5
   for _ in range(100):
    q=json.loads((t/'ledger').read_text())
    if q['spent_micros']==9 and q['reserved_micros']==0:break
    time.sleep(.01)
   assert q['spent_micros']==9 and q['reserved_micros']==0,q
   transitions=[json.loads(x) for x in (t/'proxy.log').read_text().splitlines() if 'proxy_transition' in x]
   assert not any(x['stage']=='settled_at_reserve' for x in transitions),transitions
   print('former deadline elapsed; call completed; actual usage booked once')
  finally:p.terminate();p.wait(timeout=3);up.shutdown()
