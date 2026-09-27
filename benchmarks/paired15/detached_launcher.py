#!/usr/bin/env python3
"""Start one runner in a detached process, refuse duplicate active or ambiguous records."""
import argparse,datetime,json,os,pathlib,signal,subprocess,sys
R=pathlib.Path(__file__).resolve().parent
p=argparse.ArgumentParser();p.add_argument('--out',required=True);p.add_argument('--ledger',required=True);p.add_argument('--bridge',required=True);p.add_argument('--port',type=int,default=18748);p.add_argument('--id');p.add_argument('--harness',choices=['HAIRSPRING','OpenHands']);p.add_argument('--fake',action='store_true');p.add_argument('--delay-seconds',type=int,default=0);a=p.parse_args()
root=pathlib.Path(a.out).resolve();root.mkdir(parents=True,exist_ok=True)
lock=root/'active.json'
if lock.exists():
 old=json.loads(lock.read_text());pid=old.get('pid')
 if pid:
  try:os.kill(pid,0)
  except ProcessLookupError:pass
  else:raise SystemExit(f'detached runner already active PID {pid}; do not duplicate')
 raise SystemExit(f'prior detached record exists at {lock}; inspect before any re-launch')
cmd=[sys.executable,str(R/'runner.py'),'--out',str(root/'runs'),'--ledger',a.ledger,'--bridge',a.bridge,'--port',str(a.port)]
if a.id:cmd+=['--id',a.id]
if a.harness:cmd+=['--harness',a.harness]
if a.fake:cmd.append('--fake')
if a.delay_seconds:cmd=[sys.executable,'-c','import time,subprocess,sys;time.sleep(int(sys.argv[1]));raise SystemExit(subprocess.run(sys.argv[2:]).returncode)',str(a.delay_seconds),*cmd]
log=(root/'runner.log').open('a');err=(root/'runner.err').open('a')
proc=subprocess.Popen(cmd,stdin=subprocess.DEVNULL,stdout=log,stderr=err,start_new_session=True,close_fds=True)
record={'pid':proc.pid,'started_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'command':cmd,'out':str(root)}
t=lock.with_suffix('.tmp');t.write_text(json.dumps(record,indent=2)+'\n');t.replace(lock)
print(json.dumps({'pid':proc.pid,'record':str(lock),'log':str(root/'runner.log'),'stderr':str(root/'runner.err')}))
