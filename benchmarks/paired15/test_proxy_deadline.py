"""A continuous stream survives the former total deadline and settles actual usage."""
import json,os,pathlib,socket,subprocess,sys,tempfile,threading,time,urllib.request
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
R=pathlib.Path(__file__).resolve().parent
class Up(BaseHTTPRequestHandler):
 def log_message(self,*a):pass
 def do_POST(self):
  self.rfile.read(int(self.headers['Content-Length']))
  self.send_response(200);self.send_header('Content-Type','text/event-stream');self.end_headers()
  for _ in range(8):
   self.wfile.write(b'data: {"choices":[]}\n\n');self.wfile.flush();time.sleep(.08)
  self.wfile.write(b'data: {"usage":{"prompt_tokens":10,"completion_tokens":5},"choices":[]}\n\ndata: [DONE]\n\n');self.wfile.flush()
with tempfile.TemporaryDirectory() as d:
 t=pathlib.Path(d);key=t/'key';key.write_text('dummy');key.chmod(0o600)
 up=ThreadingHTTPServer(('127.0.0.1',0),Up);threading.Thread(target=up.serve_forever,daemon=True).start()
 sock=socket.socket();sock.bind(('127.0.0.1',0));port=sock.getsockname()[1];sock.close()
 env={**os.environ,'HS_PROXY_TEST_UPSTREAM':'1','HS_PROXY_TEST_DEADLINE_SECS':'.25'}
 with (t/'proxy.log').open('w') as log:
  p=subprocess.Popen([sys.executable,str(R/'spend_proxy.py'),'--key-file',str(key),'--ledger',str(t/'ledger'),'--port',str(port),'--upstream',f'http://127.0.0.1:{up.server_port}'],env=env,stdout=log,stderr=subprocess.STDOUT)
  try:
   for _ in range(100):
    try:
     with socket.create_connection(('127.0.0.1',port),timeout=.1):break
    except OSError:time.sleep(.02)
   body=json.dumps({'model':'deepseek-flash','stream':True,'stream_options':{'include_usage':True}}).encode()
   started=time.monotonic()
   data=urllib.request.urlopen(urllib.request.Request(f'http://127.0.0.1:{port}/chat/completions',data=body),timeout=3).read()
   assert time.monotonic()-started>.5 and b'data: [DONE]' in data
   q=json.loads((t/'ledger').read_text());assert q['reserved_micros']==0 and q['spent_micros']==9,q
   transitions=[json.loads(line) for line in (t/'proxy.log').read_text().splitlines() if 'proxy_transition' in line]
   assert any(v.get('stage')=='settled_usage' and v.get('outcome')=='upstream_complete' for v in transitions),transitions
   print('former total deadline crossed; full stream delivered; usage booked once')
  finally:p.terminate();p.wait(timeout=3);up.shutdown()
