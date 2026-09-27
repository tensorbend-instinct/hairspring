"""A tracked detached fake runner lives beyond any 120s orchestration call."""
import json,pathlib,subprocess,tempfile,time,os,signal
R=pathlib.Path(__file__).resolve().parent;out=R/'detached-proof-safe';out.mkdir(exist_ok=True)
if (out/'active.json').exists():raise SystemExit('proof already exists, inspect')
# No bridge call and no paid traffic: a fake runner surrogate sleeps longer than tool timeout,
# then writes a result in its own new session.
script='import pathlib,time,sys,json;time.sleep(122);pathlib.Path(sys.argv[1]).write_text(json.dumps({"survived_120_seconds":True,"pid":__import__("os").getpid()}))'
log=(out/'proof.log').open('w');p=subprocess.Popen(['python3','-c',script,str(out/'result.json')],stdin=subprocess.DEVNULL,stdout=log,stderr=subprocess.STDOUT,start_new_session=True,close_fds=True)
(out/'active.json').write_text(json.dumps({'pid':p.pid,'started_at':time.time(),'test':'detached 122-second fake surrogate','expected':str(out/'result.json')}))
print('detached fake PID',p.pid,'PPID from /proc:',pathlib.Path(f'/proc/{p.pid}/status').read_text().split('PPid:')[1].splitlines()[0].strip())
