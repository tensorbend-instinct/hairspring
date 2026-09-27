#!/usr/bin/env python3
"""Supervise public quick tunnel, record an external page+snapshot soak.

Does not expose private data, publish links, or touch the paid benchmark runner.
"""
import json,os,re,signal,subprocess,time,pathlib
ROOT=pathlib.Path('/tmp/hs-tunnel-watch'); ROOT.mkdir(exist_ok=True)
REPO=pathlib.Path('/home/sandbox/recovery/hairspring')
STATE=ROOT/'state.json'; EVENTS=ROOT/'events.jsonl'; LOG=ROOT/'cloudflared.log'
CHILD=None; STOP=False

def stop(*_):
 global STOP
 STOP=True
signal.signal(signal.SIGTERM,stop);signal.signal(signal.SIGINT,stop)

def emit(event,**kw):
 record={'time':time.time(),'event':event,**kw}
 with EVENTS.open('a') as f:f.write(json.dumps(record)+'\n')

def launch():
 global CHILD
 with LOG.open('w') as f:
  CHILD=subprocess.Popen(['/tmp/hs-cloudflared','tunnel','--protocol','http2','--url','http://127.0.0.1:18763','--no-autoupdate'],stdout=f,stderr=subprocess.STDOUT,stdin=subprocess.DEVNULL,start_new_session=True)
 emit('launch',pid=CHILD.pid)

def url():
 try: text=LOG.read_text()
 except FileNotFoundError:return None
 m=re.search(r'https://[a-z-]+\.trycloudflare\.com',text)
 return m.group() if m else None

def check(u,path,kind):
 p=subprocess.run(['curl','--retry','0','--max-time','7','-sS','-o','/dev/null','-w','%{http_code}',u+path],capture_output=True,text=True,timeout=9)
 return p.returncode==0 and p.stdout=='200'

# Take over the previously launched HTTP/2 tunnel without needless URL churn.
existing=json.loads(STATE.read_text()).get('pid',0) if STATE.exists() else 0
if pathlib.Path(f'/proc/{existing}').exists():
 class Existing:
  pid=existing
  def poll(self):return None if pathlib.Path(f'/proc/{self.pid}').exists() else 1
  def terminate(self):os.kill(self.pid,signal.SIGTERM)
 CHILD=Existing()
 LOG=pathlib.Path('/tmp/hs-live-tunnel-http2.log')
 emit('adopt',pid=existing)
else:launch()
current=None; first_ok=None; consecutive_failures=0; checks=0; published=None
while not STOP:
 if CHILD.poll() is not None:
  emit('dead',pid=CHILD.pid);launch();current=None;first_ok=None;consecutive_failures=0
 u=url()
 if u!=current:
  if published:
   # A published URL cannot outlive its connector. Withdraw it before switching.
   try:
    work=ROOT/'publish'
    if not work.exists():subprocess.run(['git','clone','--quiet','--depth','1','https://github.com/tensorbend-instinct/hairspring.git',str(work)],check=True,timeout=30)
    subprocess.run(['git','pull','--ff-only','--quiet'],cwd=work,check=True,timeout=30)
    page=work/'docs/run/index.html';txt=page.read_text()
    txt=re.sub(r'<p id="live-status">.*?</p>','<p id="live-status">Live view is being checked. The published snapshots below remain available.</p>',txt,count=1)
    page.write_text(txt);subprocess.run(['git','add','docs/run/index.html'],cwd=work,check=True)
    subprocess.run(['git','-c','user.name=Instinct','-c','user.email=instinct@users.noreply.github.com','commit','-m','Withdraw stale tunnel link'],cwd=work,check=True,stdout=subprocess.DEVNULL)
    subprocess.run(['git','push','origin','HEAD:main'],cwd=work,check=True,timeout=30,stdout=subprocess.DEVNULL)
    published=None;emit('unpublished',reason='connector changed')
   except Exception as e:emit('unpublish_failed',error=type(e).__name__)
  if u:emit('url',url=u)
  current=u;first_ok=None;checks=0
 if u:
  page=check(u,'/','page'); snap=check(u,'/snapshot.json','snapshot')
  ok=page and snap; checks+=1
  if ok:
   consecutive_failures=0
   if first_ok is None:first_ok=time.time()
  else:
   first_ok=None;consecutive_failures+=1
  emit('check',url=u,page=page,snapshot=snap,ok=ok,checks=checks,soak_seconds=round(time.time()-first_ok,1) if first_ok else 0)
  def publish(target):
   global published
   try:
    work=ROOT/'publish'
    if not work.exists():subprocess.run(['git','clone','--quiet','--depth','1','https://github.com/tensorbend-instinct/hairspring.git',str(work)],check=True,timeout=30)
    subprocess.run(['git','pull','--ff-only','--quiet'],cwd=work,check=True,timeout=30)
    page=work/'docs/run/index.html';txt=page.read_text()
    replacement=('<p id="live-status"><a href="'+target+'/" rel="noopener noreferrer">Open the live control center</a></p>' if target else '<p id="live-status">Live view is being checked. The published snapshots below remain available.</p>')
    txt=re.sub(r'<p id="live-status">.*?</p>',replacement,txt,count=1)
    if txt!=page.read_text():
     page.write_text(txt)
     subprocess.run(['git','add','docs/run/index.html'],cwd=work,check=True)
     subprocess.run(['git','-c','user.name=Instinct','-c','user.email=instinct@users.noreply.github.com','commit','-m','Update verified live tunnel link'],cwd=work,check=True,stdout=subprocess.DEVNULL)
     subprocess.run(['git','push','origin','HEAD:main'],cwd=work,check=True,timeout=30,stdout=subprocess.DEVNULL)
    published=target;emit('published' if target else 'unpublished',url=target)
   except Exception as e:emit('publish_failed',error=type(e).__name__)
  if ok and first_ok and time.time()-first_ok>=600 and checks>=30 and (ROOT/'browser_verified_url').exists() and (ROOT/'browser_verified_url').read_text().strip()==u and published!=u:
   publish(u)
  elif not ok and published:
   publish(None)
  if consecutive_failures>=2:
   emit('restart',pid=CHILD.pid,reason='two external check failures')
   CHILD.terminate()
   for _ in range(20):
    if CHILD.poll() is not None:break
    time.sleep(.1)
   launch();current=None;first_ok=None;consecutive_failures=0
 STATE.write_text(json.dumps({'time':time.time(),'pid':CHILD.pid,'url':current,'first_ok':first_ok,'checks':checks,'consecutive_failures':consecutive_failures,'soak_seconds':round(time.time()-first_ok,1) if first_ok else 0})+'\n')
 for _ in range(15):
  if STOP:break
  time.sleep(1)
if CHILD and CHILD.poll() is None:CHILD.terminate()
emit('stopped')
