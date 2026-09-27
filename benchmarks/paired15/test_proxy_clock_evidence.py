"""Clock fields distinguish real deadline expiry from wall-clock labels."""
import json,os,pathlib,socket,subprocess,sys,tempfile,threading,time,urllib.request
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
R=pathlib.Path(__file__).resolve().parent
class Up(BaseHTTPRequestHandler):
 def log_message(self,*a):pass
 def do_POST(self):
  self.rfile.read(int(self.headers['Content-Length']))
  time.sleep(.5)
  try:self.send_response(200);self.end_headers();self.wfile.write(b'{}')
  except OSError:pass
def port():
 s=socket.socket();s.bind(('127.0.0.1',0));p=s.getsockname()[1];s.close();return p
with tempfile.TemporaryDirectory() as d:
 t=pathlib.Path(d);key=t/'key';key.write_text('dummy');key.chmod(0o600)
 up=ThreadingHTTPServer(('127.0.0.1',0),Up);threading.Thread(target=up.serve_forever,daemon=True).start()
 pp=port();env={**os.environ,'HS_PROXY_TEST_UPSTREAM':'1','HS_PROXY_TEST_DEADLINE_SECS':'.3','HS_PROXY_TEST_HEARTBEAT_FAST':'1'}
 with (t/'proxy.log').open('w') as log:
  proxy=subprocess.Popen([sys.executable,str(R/'spend_proxy.py'),'--key-file',str(key),'--ledger',str(t/'ledger'),'--port',str(pp),'--upstream',f'http://127.0.0.1:{up.server_port}'],env=env,stdout=log,stderr=subprocess.STDOUT)
  try:
   for _ in range(100):
    try:
     with socket.create_connection(('127.0.0.1',pp),timeout=.1):break
    except OSError:time.sleep(.02)
   body=json.dumps({'model':'deepseek-flash'}).encode()
   try:urllib.request.urlopen(urllib.request.Request(f'http://127.0.0.1:{pp}/chat/completions',data=body),timeout=2).read()
   except Exception:pass
   time.sleep(.15)
   rows=[json.loads(line) for line in (t/'proxy.log').read_text().splitlines() if line.startswith('{')]
   calibration=next(x for x in rows if x.get('event')=='proxy_clock_calibration')
   assert .5<calibration['wall_per_monotonic']<2,calibration
   transitions=[x for x in rows if x.get('event')=='proxy_transition']
   admitted=next(x for x in transitions if x['stage']=='admitted')
   settled=next(x for x in transitions if x['stage']=='settled_at_reserve')
   beats=[x for x in transitions if x['stage']=='watchdog_heartbeat']
   assert beats,'watchdog heartbeat absent'
   for x in transitions:
    if x['stage']=='watchdog_heartbeat' and x['elapsed_monotonic_secs'] is None:continue
    assert x['monotonic_admission']==admitted['monotonic_admission'],x
    assert x['monotonic_deadline']==admitted['monotonic_deadline'],x
    assert x['elapsed_monotonic_secs']>=0,x
   assert settled['elapsed_monotonic_secs']>=.28,settled
   assert settled['outcome']=='upstream_total_deadline',settled
   print('calibration, heartbeat, and monotonic settlement evidence present')
  finally:proxy.terminate();proxy.wait(timeout=3);up.shutdown()
