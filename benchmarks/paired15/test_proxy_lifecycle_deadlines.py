"""Pre-header, mid-body, and pre-socket stalls are not model-call kill switches."""
import json,os,pathlib,socket,subprocess,sys,tempfile,threading,time,urllib.request
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
R=pathlib.Path(__file__).resolve().parent
class Up(BaseHTTPRequestHandler):
 mode='header'
 def log_message(self,*a):pass
 def do_POST(self):
  self.rfile.read(int(self.headers['Content-Length']))
  if Up.mode=='header':time.sleep(.55)
  self.send_response(200);self.send_header('Content-Type','text/event-stream');self.end_headers()
  if Up.mode=='body':
   self.wfile.write(b'data: {"choices":[]}\n\n');self.wfile.flush();time.sleep(.55)
  self.wfile.write(b'data: {"usage":{"prompt_tokens":10,"completion_tokens":5},"choices":[]}\n\ndata: [DONE]\n\n');self.wfile.flush()
def port():
 s=socket.socket();s.bind(('127.0.0.1',0));p=s.getsockname()[1];s.close();return p
with tempfile.TemporaryDirectory(prefix='hs-proxy-phases-') as d:
 t=pathlib.Path(d);key=t/'key';key.write_text('dummy');key.chmod(0o600)
 up=ThreadingHTTPServer(('127.0.0.1',0),Up);threading.Thread(target=up.serve_forever,daemon=True).start()
 pp=port();env={**os.environ,'HS_PROXY_TEST_UPSTREAM':'1','HS_PROXY_TEST_DEADLINE_SECS':'.24'}
 with (t/'proxy.log').open('w') as log:
  proxy=None
  try:
   for index,mode in enumerate(('header','body','admitted'),1):
    if proxy:proxy.terminate();proxy.wait(timeout=3)
    Up.mode=mode
    env['HS_PROXY_TEST_ADMISSION_STALL_SECS']='.55' if mode=='admitted' else '0'
    proxy=subprocess.Popen([sys.executable,str(R/'spend_proxy.py'),'--key-file',str(key),'--ledger',str(t/'ledger'),'--port',str(pp),'--upstream',f'http://127.0.0.1:{up.server_port}'],env=env,stdout=log,stderr=subprocess.STDOUT)
    for _ in range(100):
     try:
      with socket.create_connection(('127.0.0.1',pp),timeout=.1):break
     except OSError:time.sleep(.02)
    else:raise AssertionError('proxy not listening')
    body=json.dumps({'model':'deepseek-flash','stream':True,'stream_options':{'include_usage':True}}).encode()
    start=time.monotonic()
    data=urllib.request.urlopen(urllib.request.Request(f'http://127.0.0.1:{pp}/chat/completions',data=body),timeout=3).read()
    assert time.monotonic()-start>.5 and b'data: [DONE]' in data,(mode,data)
    q=json.loads((t/'ledger').read_text());assert q['reserved_micros']==0 and q['spent_micros']==9*index,(mode,q)
   transitions=[json.loads(x) for x in (t/'proxy.log').read_text().splitlines() if 'proxy_transition' in x]
   assert len([x for x in transitions if x.get('stage')=='settled_usage'])==3,transitions
   print('all three former phase deadlines survived and settled actual usage')
  finally:
   if proxy:proxy.terminate();proxy.wait(timeout=3)
   up.shutdown()
