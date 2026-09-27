#!/usr/bin/env python3
"""Public-safe, allowlisted live run view. No raw log or ledger endpoint."""
import argparse,json,pathlib,subprocess,threading,time
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
R=pathlib.Path(__file__).resolve().parent; ROOT=R.parents[1]; DOC=ROOT/'docs/run'
p=argparse.ArgumentParser();p.add_argument('--port',type=int,required=True);p.add_argument('--proxy-log',action='append',required=True);p.add_argument('--ledger',required=True);p.add_argument('--runs',required=True);p.add_argument('--manual-id',required=True);a=p.parse_args()
lock=threading.Lock();cache={'at':0.0,'bytes':None}
STATIC={'/':('index.html','text/html; charset=utf-8'),'/style.css':('style.css','text/css; charset=utf-8'),'/app.js':('app.js','text/javascript; charset=utf-8')}
FORBIDDEN=('spent_micros','reserved_micros','ledger','/tmp/','vault','api_key','access_token','secret','key material')
def snapshot():
 with lock:
  if cache['bytes'] is not None and time.monotonic()-cache['at']<1:return cache['bytes']
  cmd=['python3',str(R/'build_control_center.py')]
  for path in a.proxy_log:cmd+=['--proxy-log',path]
  cmd+=['--ledger',a.ledger,'--runs',a.runs,'--output','/tmp/hs-live-public-snapshot.json']
  subprocess.run(cmd,check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=4)
  data=pathlib.Path('/tmp/hs-live-public-snapshot.json').read_bytes()
  low=data.lower()
  assert all(token.encode() not in low for token in FORBIDDEN)
  obj=json.loads(data);assert obj['schema']==1 and obj['summary']['calls']==len(obj['calls'])
  assert len({c['id'] for c in obj['calls']})==len(obj['calls'])
  matched=[c for c in obj['calls'] if c['id']==a.manual_id]
  assert len(matched)==1 and matched[0]['settlement']=='manual reserve'
  cache.update(at=time.monotonic(),bytes=data)
  return data
class Handler(BaseHTTPRequestHandler):
 def log_message(self,*args):pass
 def do_GET(self):
  path=self.path.split('?',1)[0]
  try:
   if path=='/snapshot.json':data=snapshot();mime='application/json; charset=utf-8'
   elif path in STATIC:
    f,mime=STATIC[path];data=(DOC/f).read_bytes()
    if path=='/':
     data=data.replace(b'published snapshots, not the provider directly.',b'live public-safe run state, refreshed every 2 seconds.').replace(b'Snapshot published',b'Live snapshot').replace(b'Checks for a new snapshot every 30 seconds',b'Polls the live run every 2 seconds').replace(b'An open call\'s age is frozen at the snapshot, not ticking live.',b'Open-call age updates each poll.')
     first=data.index(b'<section class="panel"><div class="section-title"><div><p class="eyebrow">LIVE / RUN BOX</p>')
     last=data.index(b'</section>',first)+len(b'</section>')
     archive=b'<section class="panel"><div class="section-title"><div><p class="eyebrow">ARCHIVE</p><h2>Published snapshots</h2></div><p>The durable Pages copy can lag this live view.</p></div><p><a href="https://tensorbend-instinct.github.io/hairspring/run/" rel="noopener noreferrer">Open the published archive</a></p></section>'
     data=data[:first]+archive+data[last:]
    elif path=='/app.js':data=data.replace(b'setInterval(load,30000)',b'setInterval(load,2000)')
   else:self.send_error(404);return
  except Exception:self.send_error(503,'snapshot unavailable');return
  self.send_response(200);self.send_header('Content-Type',mime);self.send_header('Cache-Control','no-store, max-age=0');self.send_header('X-Content-Type-Options','nosniff');self.send_header('Referrer-Policy','no-referrer');self.send_header('Content-Security-Policy',"default-src 'none'; style-src 'self'; script-src 'self'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; frame-ancestors 'none'");self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
ThreadingHTTPServer(('127.0.0.1',a.port),Handler).serve_forever()
