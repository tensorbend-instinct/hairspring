import json,os,pathlib,socket,subprocess,sys,tempfile,threading,time,urllib.request,urllib.error
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
ROOT=pathlib.Path(__file__).resolve().parent
class Fake(BaseHTTPRequestHandler):
    calls=0
    def log_message(self,*a):pass
    def do_POST(self):
        Fake.calls+=1
        n=int(self.headers['Content-Length']);v=json.loads(self.rfile.read(n))
        assert self.headers['Authorization']=='Bearer testing-only-key'
        if v.get('stream'):
            body=b'data: {"choices":[{"delta":{"content":"ok"}}]}\n\ndata: {"usage":{"prompt_tokens":10,"completion_tokens":5},"choices":[]}\n\ndata: [DONE]\n\n'
            ct='text/event-stream'
        elif v.get('test_missing_usage'):
            body=b'{"choices":[]}';ct='application/json'
        else:body=b'{"usage":{"prompt_tokens":10,"completion_tokens":5},"choices":[]}';ct='application/json'
        self.send_response(200);self.send_header('Content-Type',ct);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
def post(port,stream=False,**extra):
    data=json.dumps({'model':'deepseek-flash','stream':stream,**extra}).encode();r=urllib.request.Request(f'http://127.0.0.1:{port}/chat/completions',data=data,headers={'Content-Type':'application/json'})
    try:
        with urllib.request.urlopen(r,timeout=5) as resp:return resp.status,resp.read()
    except urllib.error.HTTPError as e:return e.code,e.read()
def port():
    s=socket.socket();s.bind(('127.0.0.1',0));p=s.getsockname()[1];s.close();return p
with tempfile.TemporaryDirectory() as d:
    d=pathlib.Path(d);key=d/'key';key.write_text('testing-only-key');key.chmod(0o600)
    upstream=ThreadingHTTPServer(('127.0.0.1',0),Fake);threading.Thread(target=upstream.serve_forever,daemon=True).start();base=f'http://127.0.0.1:{upstream.server_port}'
    def launch(ceiling):
        p=port();ledger=d/f'ledger-{p}.json';env=os.environ.copy();env['HS_PROXY_TEST_UPSTREAM']='1'
        proc=subprocess.Popen([sys.executable,str(ROOT/'spend_proxy.py'),'--key-file',str(key),'--ledger',str(ledger),'--port',str(p),'--upstream',base,'--ceiling-micros',str(ceiling),'--port',str(p)],env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        line=proc.stdout.readline();assert line,line
        meta=json.loads(line)
        for _ in range(100):
            try:
                with socket.create_connection(('127.0.0.1',p),timeout=.1):break
            except OSError:time.sleep(.02)
        else:raise AssertionError('proxy never listened')
        return p,ledger,proc,meta
    try:
        p,l,proc,meta=launch(150_000_000);assert meta['reserve_micros_per_call']==771860,meta
        assert post(p)[0]==200
        assert post(p,stream=True,stream_options={'include_usage':True})[0]==200
        for _ in range(100):
            try:state=json.loads(l.read_text())
            except json.JSONDecodeError:time.sleep(.02);continue
            if state.get('reserved_micros')==0:break
            time.sleep(.02)
        assert state['spent_micros']==18 and state['reserved_micros']==0 and not state['blocked'],state
        print('plain + SSE final usage: two calls, $0.000018 billed, no reserve remaining')
        proc.terminate();proc.wait(timeout=5)
        p,l,proc,meta=launch(771_859);before=Fake.calls
        code,_=post(p);assert code==429 and Fake.calls==before
        print('admission reserve: call denied before upstream when cap below reserve')
        proc.terminate();proc.wait(timeout=5)
        p,l,proc,meta=launch(2_000_000);assert post(p,test_missing_usage=True)[0]==200
        for _ in range(100):
            try:state=json.loads(l.read_text())
            except json.JSONDecodeError:time.sleep(.02);continue
            if state.get('reserved_micros')==0:break
            time.sleep(.02)
        assert not state['blocked'] and state['reserved_micros']==0 and state['spent_micros']==771860
        before=Fake.calls;assert post(p)[0]==200 and Fake.calls==before+1
        print('missing usage: reserve booked as spent and later calls admitted')
        proc.terminate();proc.wait(timeout=5)
    finally:
        upstream.shutdown()
