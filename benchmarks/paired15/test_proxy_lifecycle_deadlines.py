"""Offline complete-exchange deadlines: pre-header, header, body, and late worker."""
import http.client,json,os,pathlib,socket,subprocess,sys,tempfile,threading,time,urllib.request,urllib.error
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
R=pathlib.Path(__file__).resolve().parent
class Up(BaseHTTPRequestHandler):
 mode='header';seen=threading.Event();release=threading.Event()
 def log_message(self,*a):pass
 def do_POST(self):
  self.rfile.read(int(self.headers['Content-Length']));Up.seen.set()
  if Up.mode in ('header','admitted'):Up.release.wait(2)
  if Up.mode=='body':
   self.send_response(200);self.send_header('Content-Type','text/event-stream');self.end_headers()
   self.wfile.write(b'data: {"choices":[]}\n\n');self.wfile.flush();Up.release.wait(2)
  body=b'data: {"usage":{"prompt_tokens":10,"completion_tokens":5},"choices":[]}\n\ndata: [DONE]\n\n'
  if Up.mode!='admitted' or Up.release.is_set():
   try:
    if Up.mode!='body':self.send_response(200);self.send_header('Content-Length',str(len(body)));self.send_header('Content-Type','text/event-stream');self.end_headers()
    self.wfile.write(body);self.wfile.flush()
   except OSError:pass

def port():
 s=socket.socket();s.bind(('127.0.0.1',0));p=s.getsockname()[1];s.close();return p
with tempfile.TemporaryDirectory(prefix='hs-proxy-phases-') as d:
 t=pathlib.Path(d);key=t/'key';key.write_text('dummy');key.chmod(0o600)
 up=ThreadingHTTPServer(('127.0.0.1',0),Up);threading.Thread(target=up.serve_forever,daemon=True).start()
 pp=port();env={**os.environ,'HS_PROXY_TEST_UPSTREAM':'1','HS_PROXY_TEST_DEADLINE_SECS':'.24'}
 with (t/'proxy.log').open('w') as log:
  proxy=subprocess.Popen([sys.executable,str(R/'spend_proxy.py'),'--key-file',str(key),'--ledger',str(t/'ledger'),'--port',str(pp),'--upstream',f'http://127.0.0.1:{up.server_port}'],env=env,stdout=log,stderr=subprocess.STDOUT)
  try:
   for _ in range(100):
    try:
     with socket.create_connection(('127.0.0.1',pp),timeout=.1):break
    except OSError:time.sleep(.02)
   else:raise AssertionError('proxy not listening')
   for mode in ('header','body','admitted'):
    Up.mode=mode;Up.seen.clear();Up.release.clear()
    # Admission test needs a fresh proxy with its worker delayed before it
    # opens an upstream socket. The environment hook is test-upstream-only.
    if mode=='admitted':
     proxy.terminate();proxy.wait(timeout=3)
     env['HS_PROXY_TEST_ADMISSION_STALL_SECS']='.6'
     proxy=subprocess.Popen([sys.executable,str(R/'spend_proxy.py'),'--key-file',str(key),'--ledger',str(t/'ledger'),'--port',str(pp),'--upstream',f'http://127.0.0.1:{up.server_port}'],env=env,stdout=log,stderr=subprocess.STDOUT)
     for _ in range(100):
      try:
       with socket.create_connection(('127.0.0.1',pp),timeout=.1):break
      except OSError:time.sleep(.02)
     else:raise AssertionError('proxy restart failed')
    body=json.dumps({'model':'deepseek-flash','stream':True,'stream_options':{'include_usage':True}}).encode()
    req=urllib.request.Request(f'http://127.0.0.1:{pp}/chat/completions',data=body)
    start=time.monotonic()
    try:
     with urllib.request.urlopen(req,timeout=3) as response:response.read()
     raise AssertionError('incomplete exchange accepted')
    except (urllib.error.HTTPError,http.client.IncompleteRead,OSError) as e:
     print(mode,'client aborted',type(e).__name__)
    elapsed=time.monotonic()-start;assert elapsed<1.0,(mode,elapsed)
    if mode=='admitted':assert not Up.seen.is_set(),'admission stall reached upstream before deadline'
    for _ in range(100):
     q=json.loads((t/'ledger').read_text() or '{}')
     if q.get('reserved_micros')==0 and q.get('spent_micros')==771860*('header','body','admitted').index(mode)+771860:break
     time.sleep(.01)
    assert q['reserved_micros']==0 and q['spent_micros']==771860*(('header','body','admitted').index(mode)+1),(mode,q)
    before=q['spent_micros'];Up.release.set();time.sleep(.7 if mode=='admitted' else .12)
    q=json.loads((t/'ledger').read_text());assert q['spent_micros']==before and q['reserved_micros']==0,(mode,q)
    transitions=[json.loads(x) for x in (t/'proxy.log').read_text().splitlines() if 'proxy_transition' in x]
    assert len([x for x in transitions if x.get('stage')=='settled_at_reserve'])==('header','body','admitted').index(mode)+1
   print('all phase-stall deadlines booked reserve exactly once, late worker had no effect')
  finally:Up.release.set();proxy.terminate();proxy.wait(timeout=3);up.shutdown()
