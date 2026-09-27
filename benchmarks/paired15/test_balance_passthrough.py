import json,pathlib,socket,subprocess,tempfile,threading,urllib.request,os
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
R=pathlib.Path('/home/sandbox/recovery/hairspring/benchmarks/paired15')
class Fake(BaseHTTPRequestHandler):
 def log_message(self,*args):pass
 def do_GET(self):
  assert self.path=='/user/balance' and self.headers['Authorization']=='Bearer fake-only'
  b=json.dumps({'is_available':True,'balance_infos':[{'currency':'USD','total_balance':'123.45','granted_balance':'0','topped_up_balance':'123.45'}]}).encode();self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(b)));self.end_headers();self.wfile.write(b)
def port():
 s=socket.socket();s.bind(('127.0.0.1',0));p=s.getsockname()[1];s.close();return p
with tempfile.TemporaryDirectory(prefix='hs-balance-fake-') as d:
 t=pathlib.Path(d);key=t/'key';key.write_text('fake-only');key.chmod(0o600)
 up=ThreadingHTTPServer(('127.0.0.1',0),Fake);threading.Thread(target=up.serve_forever,daemon=True).start()
 p=subprocess.Popen(['python3',str(R/'spend_proxy.py'),'--key-file',str(key),'--ledger',str(t/'ledger'),'--port',str(port()),'--upstream',f'http://127.0.0.1:{up.server_port}'],env={**os.environ,'HS_PROXY_TEST_UPSTREAM':'1'},stdout=subprocess.PIPE)
 try:
  proxy_port=json.loads(p.stdout.readline())['port']
  with urllib.request.urlopen(f'http://127.0.0.1:{proxy_port}/user/balance',timeout=3) as r:
   body=json.loads(r.read());assert r.status==200 and body['balance_infos'][0]['total_balance']=='123.45'
  print('balance passthrough 200, USD 123.45, provider GET authenticated, no model calls')
 finally:p.terminate();p.wait(timeout=3);up.shutdown()
