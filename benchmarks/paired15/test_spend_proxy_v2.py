"""Offline accounting: disconnect keeps upstream; missing usage/error books reserve."""
import json,os,pathlib,socket,subprocess,tempfile,threading,time,urllib.request,urllib.error
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
R=pathlib.Path('/home/sandbox/recovery/hairspring/benchmarks/paired15')
class Up(BaseHTTPRequestHandler):
 mode='normal';done=threading.Event();calls=0
 def log_message(self,*a):pass
 def do_POST(self):
  self.rfile.read(int(self.headers['Content-Length']));Up.calls+=1
  if Up.mode=='error':self.send_error(503);Up.done.set();return
  if Up.mode=='disconnect':time.sleep(.3)
  b=b'{"usage":{"prompt_tokens":10,"completion_tokens":5},"choices":[]}' if Up.mode!='missing' else b'{"choices":[]}'
  self.send_response(200);self.send_header('Content-Length',str(len(b)));self.end_headers();self.wfile.write(b);Up.done.set()
def port():
 s=socket.socket();s.bind(('127.0.0.1',0));p=s.getsockname()[1];s.close();return p
with tempfile.TemporaryDirectory() as d:
 t=pathlib.Path(d);key=t/'key';key.write_text('dummy');key.chmod(0o600)
 up=ThreadingHTTPServer(('127.0.0.1',0),Up);threading.Thread(target=up.serve_forever,daemon=True).start()
 proxy=subprocess.Popen(['python3',str(R/'spend_proxy.py'),'--key-file',str(key),'--ledger',str(t/'ledger'),'--port',str(port()),'--upstream',f'http://127.0.0.1:{up.server_port}'],env={**os.environ,'HS_PROXY_TEST_UPSTREAM':'1'},stdout=(t/'proxy.log').open('w'),stderr=subprocess.STDOUT)
 try:
  for _ in range(100):
   try:
    import re
    m=re.search(r'"port": (\d+)',(t/'proxy.log').read_text())
    if m:break
   except OSError:pass
   time.sleep(.02)
  p=int(m.group(1));body=json.dumps({'model':'deepseek-flash'}).encode()
  for mode,expect in [('normal',9),('disconnect',9),('missing',771860),('error',771860)]:
   Up.mode=mode;Up.done.clear();before=json.loads((t/'ledger').read_text() or '{}').get('spent_micros',0)
   if mode=='disconnect':
    s=socket.create_connection(('127.0.0.1',p));s.sendall(b'POST /chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: '+str(len(body)).encode()+b'\r\n\r\n'+body);s.close()
   else:
    req=urllib.request.Request(f'http://127.0.0.1:{p}/chat/completions',data=body)
    try:urllib.request.urlopen(req,timeout=4).read()
    except urllib.error.HTTPError:pass
   for _ in range(200):

    try:q=json.loads((t/'ledger').read_text() or '{}')
    except json.JSONDecodeError:time.sleep(.02);continue
    if q.get('spent_micros',0)-before==expect and q.get('reserved_micros')==0:break
    time.sleep(.02)
   assert q['spent_micros']-before==expect and q['reserved_micros']==0 and not q['blocked'],(mode,q)
   print(mode,'booked',expect,'reserve 0, unblocked')
  logs=(t/'proxy.log').read_text();assert 'client_disconnected' in logs and 'settled_usage' in logs and 'settled_at_reserve' in logs
  print('transition logging recorded without credentials')
 finally:proxy.terminate();proxy.wait(timeout=3);up.shutdown()
