#!/usr/bin/env python3
"""Offline proxy-only egress proof with a fake local provider and fake key."""
import json,os,pathlib,socket,subprocess,tempfile,threading,time,urllib.request
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
ROOT=pathlib.Path(__file__).resolve().parent
class Fake(BaseHTTPRequestHandler):
    calls=0
    def log_message(self,*a):pass
    def do_POST(self):
        Fake.calls+=1
        assert self.path=='/chat/completions' and self.headers['Authorization']=='Bearer fake-test-key'
        body=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        assert body['model']=='deepseek-flash'
        reply=json.dumps({'choices':[{'message':{'content':'fake'}}],'usage':{'prompt_tokens':10,'completion_tokens':5}}).encode()
        self.send_response(200);self.send_header('Content-Length',str(len(reply)));self.end_headers();self.wfile.write(reply)
def port():
    s=socket.socket();s.bind(('127.0.0.1',0));p=s.getsockname()[1];s.close();return p
with tempfile.TemporaryDirectory(prefix='hs-proxy-isolation-') as tmp:
    t=pathlib.Path(tmp);(t/'work').mkdir();(t/'bridge').mkdir();(t/'hidden').mkdir()
    (t/'hidden/secret').write_text('hidden')
    key=t/'hidden/key';key.write_text('fake-test-key');key.chmod(0o600)
    upstream=ThreadingHTTPServer(('127.0.0.1',0),Fake);threading.Thread(target=upstream.serve_forever,daemon=True).start()
    pp=port();env={**os.environ,'HS_PROXY_TEST_UPSTREAM':'1'}
    proxy=subprocess.Popen(['python3',str(ROOT/'spend_proxy.py'),'--key-file',str(key),'--ledger',str(t/'ledger.json'),'--port',str(pp),'--upstream',f'http://127.0.0.1:{upstream.server_port}','--ceiling-micros','771860'],env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    assert proxy.stdout.readline()
    bridge=subprocess.Popen(['socat',f'UNIX-LISTEN:{t}/bridge/proxy.sock,fork',f'TCP:127.0.0.1:{pp}'],stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
    try:
        for _ in range(100):
            if (t/'bridge/proxy.sock').exists():break
            time.sleep(.02)
        script='''
import json,pathlib,socket,subprocess,time,urllib.error,urllib.request
assert not pathlib.Path('/host-hidden').exists()
assert not pathlib.Path('/tmp/hs-audit').exists()
s=socket.socket();assert s.connect_ex(('1.1.1.1',443))!=0
bridge=subprocess.Popen(['socat','TCP-LISTEN:18748,bind=127.0.0.1,reuseaddr,fork','UNIX-CONNECT:/bridge/proxy.sock'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
try:
    for _ in range(100):
        try:
            with socket.create_connection(('127.0.0.1',18748),timeout=.1):break
        except OSError:time.sleep(.02)
    data=b'{"model":"deepseek-flash","messages":[{"role":"user","content":"offline"}]}'
    req=urllib.request.Request('http://127.0.0.1:18748/v1/chat/completions',data=data,headers={'Content-Type':'application/json'})
    with urllib.request.urlopen(req,timeout=5) as r: assert json.load(r)['choices'][0]['message']['content']=='fake'
    try:urllib.request.urlopen(req,timeout=5)
    except urllib.error.HTTPError as e:assert e.code==429
    else:raise AssertionError('ceiling failed')
    print('agent namespace: one request crossed only mounted socket to proxy; second denied; no direct egress or hidden host path')
finally:bridge.terminate();bridge.wait(timeout=3)
'''
        args=['bwrap','--unshare-net','--ro-bind','/usr','/usr','--ro-bind','/bin','/bin','--ro-bind','/lib','/lib','--ro-bind','/lib64','/lib64','--proc','/proc','--dev','/dev','--tmpfs','/tmp','--dir','/workspace','--bind',str(t/'work'),'/workspace','--dir','/bridge','--bind',str(t/'bridge'),'/bridge','--chdir','/workspace','--setenv','PATH','/usr/bin:/bin','--','python3','-c',script]
        run=subprocess.run(args,capture_output=True,text=True,timeout=20);print(run.stdout,end='');assert run.returncode==0,run.stderr
        state=json.loads((t/'ledger.json').read_text());assert state['spent_micros']==9 and state['reserved_micros']==0 and Fake.calls==1,(state,Fake.calls)
        print('host ledger: one billed fake call, zero outstanding reserve; no agent access to key file')
    finally:
        bridge.terminate();proxy.terminate();upstream.shutdown();bridge.wait(timeout=3);proxy.wait(timeout=3)
