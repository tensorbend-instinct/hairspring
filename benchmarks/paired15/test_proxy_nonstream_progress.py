"""An invalid non-SSE body is measured and booked conservatively, without a timer."""
import json,os,pathlib,socket,subprocess,sys,tempfile,threading,time,urllib.request
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
R=pathlib.Path(__file__).resolve().parent
class Up(BaseHTTPRequestHandler):
 def log_message(self,*a):pass
 def do_POST(self):
  self.rfile.read(int(self.headers['Content-Length']))
  self.send_response(200);self.send_header('Content-Type','application/json');self.end_headers()
  self.wfile.write(b'{"partial":');self.wfile.flush();time.sleep(.55)
def port():
 s=socket.socket();s.bind(('127.0.0.1',0));p=s.getsockname()[1];s.close();return p
with tempfile.TemporaryDirectory() as d:
 t=pathlib.Path(d);key=t/'key';key.write_text('dummy');key.chmod(0o600)
 up=ThreadingHTTPServer(('127.0.0.1',0),Up);threading.Thread(target=up.serve_forever,daemon=True).start()
 pp=port();env={**os.environ,'HS_PROXY_TEST_UPSTREAM':'1','HS_PROXY_TEST_DEADLINE_SECS':'.35'}
 with (t/'proxy.log').open('w') as log:
  proxy=subprocess.Popen([sys.executable,str(R/'spend_proxy.py'),'--key-file',str(key),'--ledger',str(t/'ledger'),'--port',str(pp),'--upstream',f'http://127.0.0.1:{up.server_port}'],env=env,stdout=log,stderr=subprocess.STDOUT)
  try:
   for _ in range(100):
    try:
     with socket.create_connection(('127.0.0.1',pp),timeout=.1):break
    except OSError:time.sleep(.02)
   body=json.dumps({'model':'deepseek-flash','stream':False}).encode()
   try:urllib.request.urlopen(urllib.request.Request(f'http://127.0.0.1:{pp}/chat/completions',data=body),timeout=2).read()
   except Exception:pass
   time.sleep(.05)
   es=[json.loads(x) for x in (t/'proxy.log').read_text().splitlines() if 'proxy_transition' in x]
   assert any(e['stage']=='upstream_first_byte' and e['upstream_bytes']==11 for e in es),es
   assert any(e['stage']=='settled_at_reserve' and e['outcome']=='upstream_error' for e in es),es
   assert any(e['stage']=='upstream_body_complete' and e['upstream_bytes']==11 for e in es),es
   assert json.loads((t/'ledger').read_text())['reserved_micros']==0
   print('invalid non-SSE response: bytes measured, parse failed, reserve booked')
  finally:proxy.terminate();proxy.wait(timeout=3);up.shutdown()
