"""RED: sequential non-SSE model calls reuse the same upstream TCP connection."""
import json,os,pathlib,socket,subprocess,sys,tempfile,threading,time,urllib.request
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
R=pathlib.Path(__file__).resolve().parent
connections=[]
class Up(BaseHTTPRequestHandler):
 protocol_version='HTTP/1.1'
 def log_message(self,*a): pass
 def do_POST(self):
  self.rfile.read(int(self.headers['Content-Length']))
  connections.append((self.client_address[1],self.headers.get('Connection')))
  b=b'{"usage":{"prompt_tokens":3,"completion_tokens":2}}'
  self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(b)));self.end_headers();self.wfile.write(b);self.wfile.flush()
  self.close_connection=True
def port():
 s=socket.socket();s.bind(('127.0.0.1',0));p=s.getsockname()[1];s.close();return p
with tempfile.TemporaryDirectory() as d:
 t=pathlib.Path(d);key=t/'key';key.write_text('dummy');key.chmod(0o600)
 up=ThreadingHTTPServer(('127.0.0.1',0),Up);threading.Thread(target=up.serve_forever,daemon=True).start()
 pp=port();env={**os.environ,'HS_PROXY_TEST_UPSTREAM':'1','HS_PROXY_TEST_DEADLINE_SECS':'3'}
 with (t/'proxy.log').open('w') as log:
  proxy=subprocess.Popen([sys.executable,str(R/'spend_proxy.py'),'--key-file',str(key),'--ledger',str(t/'ledger'),'--port',str(pp),'--upstream',f'http://127.0.0.1:{up.server_port}'],env=env,stdout=log,stderr=subprocess.STDOUT)
  try:
   for _ in range(100):
    try:
     with socket.create_connection(('127.0.0.1',pp),timeout=.1):break
    except OSError:time.sleep(.02)
   body=json.dumps({'model':'deepseek-flash','stream':False}).encode()
   for _ in range(2):
    with urllib.request.urlopen(urllib.request.Request(f'http://127.0.0.1:{pp}/chat/completions',data=body),timeout=4) as r:assert json.loads(r.read())['usage']['completion_tokens']==2
   assert len(connections)==2,connections
   assert connections[0][0]!=connections[1][0],f'provider closed idle socket, proxy did not recover: {connections}'
   print('provider-closed idle socket recovered on fresh TCP connection',connections)
  finally:proxy.terminate();proxy.wait(timeout=3);up.shutdown()
