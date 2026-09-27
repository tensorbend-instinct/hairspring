#!/usr/bin/env python3
"""Local DeepSeek-only metering proxy. No request shaping. Never log credentials.

A concurrent call reserves max documented Flash cost at peak prices; unknown usage
books the entire reserve as spent and records the uncertainty. A configured cap is across processes
using one ledger file and flock. The upstream URL must be the direct provider.
"""
import argparse,fcntl,json,os,pathlib,threading,queue,urllib.request,urllib.error,secrets,hmac,html,urllib.parse,time,socket
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from decimal import Decimal,ROUND_UP
P=argparse.ArgumentParser();P.add_argument('--key-file');P.add_argument('--admin-port',type=int);P.add_argument('--admin-token-file');P.add_argument('--ledger',required=True);P.add_argument('--port',type=int,default=18748);P.add_argument('--upstream',default='https://api.deepseek.com');P.add_argument('--ceiling-micros',type=int,default=150_000_000);A=P.parse_args()
if A.upstream.rstrip('/')!='https://api.deepseek.com' and os.environ.get('HS_PROXY_TEST_UPSTREAM')!='1':raise SystemExit('only direct DeepSeek upstream permitted')
if bool(A.key_file)==bool(A.admin_port):raise SystemExit('select exactly one secret input: key-file or admin-port')
K=pathlib.Path(A.key_file) if A.key_file else None
if K and K.stat().st_mode & 0o077:raise SystemExit('key file must be owner-only')
L=pathlib.Path(A.ledger);L.parent.mkdir(parents=True,exist_ok=True);L.touch(exist_ok=True)
ADMIN_TOKEN=secrets.token_urlsafe(32) if A.admin_port else None
MEMORY_KEY=None
KEY_LOCK=threading.Lock()
if A.admin_port:
    if not A.admin_token_file:raise SystemExit('admin token file required')
    tf=pathlib.Path(A.admin_token_file)
    if tf.exists():raise SystemExit('admin token file already exists')
    fd=os.open(str(tf),os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
    with os.fdopen(fd,'w') as f:f.write(ADMIN_TOKEN)

# 1M input at $0.30/M, 393216 output at $1.20/M (rounded up).
RESERVE=int((Decimal(1_000_000)*Decimal('.30')+Decimal(393_216)*Decimal('1.20')).to_integral_value(rounding=ROUND_UP))
# Rates are USD micros per million tokens, so multiplication above is USD micros.
UPSTREAM_DEADLINE_SECS = 1020.0
if os.environ.get('HS_PROXY_TEST_UPSTREAM') == '1':
    UPSTREAM_DEADLINE_SECS = float(os.environ.get('HS_PROXY_TEST_DEADLINE_SECS', '1020'))
LOCK=threading.Lock()
SETTLED_REQUESTS=set()
REQUEST_CLOCKS={}
def mutate(delta=0,settle=None,label=None,request_id=None):
    # flock serializes independent file descriptions across threads and processes.
    # Settlement must never acquire the coordinator's in-process LOCK.
    with L.open('r+') as f:
        fcntl.flock(f,fcntl.LOCK_EX);f.seek(0)
        try:s=json.loads(f.read() or '{}')
        except json.JSONDecodeError:raise RuntimeError('unreadable ledger')
        s.setdefault('spent_micros',0);s.setdefault('reserved_micros',0);s.setdefault('blocked',False)
        if delta:
            if s['blocked'] or s['spent_micros']+s['reserved_micros']+delta>A.ceiling_micros:
                result=False
            else:s['reserved_micros']+=delta;result=True
        elif settle is not None:
            if request_id is None:raise RuntimeError('settlement requires request ID')
            if request_id in SETTLED_REQUESTS:return False
            if s['reserved_micros']<RESERVE:raise RuntimeError('reserve underflow')
            s['reserved_micros']-=RESERVE;s['spent_micros']+=settle
            SETTLED_REQUESTS.add(request_id)
            if label:s.setdefault('adjustments',[]).append({'at':time.time(),'request_id':request_id,'micros':settle,'label':label})
            if s['spent_micros']+s['reserved_micros']>A.ceiling_micros:s['blocked']=True
            result=True
        else:result=s
        f.seek(0);f.write(json.dumps(s));f.truncate();f.flush();os.fsync(f.fileno());fcntl.flock(f,fcntl.LOCK_UN)
        return result

def transition(request_id,stage,**fields):
    # Never log request body, authorization header, or response content.
    now=time.monotonic()
    admission,deadline=REQUEST_CLOCKS.get(request_id,(None,None))
    print(json.dumps({'event':'proxy_transition','request_id':request_id,'stage':stage,'at':time.time(),
        'monotonic_at':now,'monotonic_admission':admission,'monotonic_deadline':deadline,
        'elapsed_monotonic_secs':None if admission is None else now-admission,**fields}),flush=True)

def cost(usage):
    if not isinstance(usage,dict):return None
    p=usage.get('prompt_tokens');c=usage.get('completion_tokens')
    if not all(isinstance(x,int) and x>=0 for x in (p,c)):return None
    if p>1_000_000 or c>393_216:return None
    return int((Decimal(p)*Decimal('.30')+Decimal(c)*Decimal('1.20')).to_integral_value(rounding=ROUND_UP))

class Handler(BaseHTTPRequestHandler):
    protocol_version="HTTP/1.1"
    def log_message(self,*args):pass
    def do_GET(self):
        if self.path!='/user/balance':self.send_error(404);return
        with KEY_LOCK:
            key=K.read_text().strip() if K else MEMORY_KEY
        if not key:self.send_error(503,'credential not loaded');return
        req=urllib.request.Request(A.upstream.rstrip('/')+'/user/balance',headers={'Authorization':'Bearer '+key})
        try:
            with urllib.request.urlopen(req,timeout=20) as resp:
                # Read-only balance passthrough for measured spend cross-check.
                data=resp.read(8192)
                self.send_response(resp.status);self.send_header('Content-Type',resp.headers.get('Content-Type','application/json'));self.send_header('Cache-Control','no-store');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
        except urllib.error.HTTPError as e:self.send_error(e.code,'provider authentication failed')
        except Exception:self.send_error(502,'provider preflight unavailable')
    def do_POST(self):
        if self.path not in ('/chat/completions','/v1/chat/completions'):
            self.send_error(404);return
        n=self.headers.get('Content-Length')
        if not n or not n.isdigit():self.send_error(411);return
        if int(n)>16_000_000:self.send_error(413,'request body too large');return
        body=self.rfile.read(int(n))
        try:v=json.loads(body)
        except (ValueError,UnicodeDecodeError):self.send_error(400);return
        if not isinstance(v,dict):self.send_error(400,'JSON object required');return
        if v.get('model')!='deepseek-flash':self.send_error(400,'direct Flash model required');return
        if v.get('stream') and (not isinstance(v.get('stream_options'),dict) or v['stream_options'].get('include_usage') is not True):
            self.send_error(400,'stream must request final usage');return
        if not K and MEMORY_KEY is None:self.send_error(503,'credential not loaded');return
        request_id=secrets.token_hex(8)
        if not mutate(delta=RESERVE):
            transition(request_id,'admission_denied');self.send_error(429,'mission spend ceiling');return
        admitted_monotonic=time.monotonic()
        deadline=admitted_monotonic+UPSTREAM_DEADLINE_SECS
        REQUEST_CLOCKS[request_id]=(admitted_monotonic,deadline)
        transition(request_id,'admitted',reserve_micros=RESERVE,request_bytes=len(body),message_bytes=sum(len(json.dumps(m).encode()) for m in v.get('messages',[]) if isinstance(m,dict)),stream=bool(v.get('stream')))
        # Only the coordinator touches the client or ledger. The upstream
        # worker can stall anywhere (DNS/connect/TLS/headers/body), so socket
        # inactivity timeouts cannot enforce an absolute request deadline.
        events=queue.Queue(maxsize=32);stop=threading.Event()
        settled=threading.Event();usage_box={'snapshot':(None,False)}
        upstream_socket_box={'value':None}
        # The watchdog never waits for coordinator progress or client I/O.
        # It alone guarantees settlement even if the coordinator is wedged.
        def settle_once(outcome):
            # One immutable tuple assignment publishes usage without a coordinator lock.
            snapshot=usage_box['snapshot']
            actual=cost(snapshot[0]) if snapshot[1] else None
            amount=RESERVE if actual is None else actual
            if mutate(settle=amount,label='no usage, booked at reserve' if actual is None else None,request_id=request_id):
                transition(request_id,'settled_at_reserve' if actual is None else 'settled_usage',micros=amount,outcome=outcome)
            settled.set()
            REQUEST_CLOCKS.pop(request_id,None)
        def heartbeat():
            # Separate observer: a slow log never holds up deadline enforcement.
            interval=.05 if os.environ.get('HS_PROXY_TEST_HEARTBEAT_FAST')=='1' else 30.0
            while not settled.wait(interval):
                transition(request_id,'watchdog_heartbeat',remaining_monotonic_secs=max(0,deadline-time.monotonic()))
                if time.monotonic()>=deadline:break
        threading.Thread(target=heartbeat,daemon=True,name='proxy-heartbeat-'+request_id).start()
        def watchdog():
            if not settled.wait(max(0,deadline-time.monotonic())):
                stop.set()
                settle_once('upstream_total_deadline')
                upstream_socket=upstream_socket_box['value']
                if upstream_socket is not None:
                    try:upstream_socket.shutdown(socket.SHUT_RDWR)
                    except OSError:pass
                    try:upstream_socket.close()
                    except OSError:pass
                try:self.connection.shutdown(2)
                except OSError:pass
                try:self.connection.close()
                except OSError:pass
        threading.Thread(target=watchdog,daemon=True,name='proxy-deadline-'+request_id).start()
        def put(kind, value=None):
            while not stop.is_set():
                try:events.put((kind,value),timeout=.1);return
                except queue.Full:pass
        # Only counts and timing; no response content. The independent progress
        # observer logs even when the upstream read is blocked on its next byte.
        progress={'bytes':0,'first_at':None,'last_at':None,'status':None}
        upstream_done=threading.Event()
        if not v.get('stream'):
            def observe_body():
                while not upstream_done.wait(5.0) and not settled.is_set():
                    transition(request_id,'upstream_body_progress',upstream_bytes=progress['bytes'],
                        first_byte_monotonic=progress['first_at'],last_byte_monotonic=progress['last_at'],
                        response_status=progress['status'])
            threading.Thread(target=observe_body,daemon=True,name='proxy-body-progress-'+request_id).start()
        def upstream():
            total_bytes=0
            try:
                if os.environ.get('HS_PROXY_TEST_UPSTREAM')=='1':
                    time.sleep(float(os.environ.get('HS_PROXY_TEST_ADMISSION_STALL_SECS','0')))
                if stop.is_set():return
                with KEY_LOCK:
                    key=K.read_text().strip() if K else MEMORY_KEY
                if not key:raise RuntimeError('no key loaded')
                req=urllib.request.Request(A.upstream.rstrip('/')+'/chat/completions',data=body,method='POST',headers={'Authorization':'Bearer '+key,'Content-Type':'application/json','Accept':'text/event-stream' if v.get('stream') else 'application/json'})
                with urllib.request.urlopen(req,timeout=UPSTREAM_DEADLINE_SECS) as resp:
                    try:upstream_socket_box['value']=resp.fp.raw._sock
                    except AttributeError:pass
                    progress['status']=resp.status
                    put('headers',(resp.status,resp.headers.get('Content-Type','application/json')))
                    if v.get('stream'):
                        # SSE is complete at [DONE], not at HTTP socket EOF.
                        # A provider can keep its connection alive indefinitely.
                        for chunk in iter(resp.readline,b''):
                            if stop.is_set():break
                            total_bytes+=len(chunk)
                            put('chunk',chunk)
                            if chunk.strip()==b'data: [DONE]':
                                transition(request_id,'upstream_sse_done',upstream_bytes=total_bytes)
                                break
                    else:
                        while not stop.is_set():
                            # read1 returns available bytes rather than waiting to fill
                            # 16 KiB, making the progress markers meaningful.
                            chunk=resp.read1(16384) if hasattr(resp,'read1') else resp.read(16384)
                            if not chunk:break
                            total_bytes+=len(chunk)
                            now=time.monotonic()
                            if progress['first_at'] is None:
                                progress['first_at']=now
                                transition(request_id,'upstream_first_byte',upstream_bytes=total_bytes)
                            progress['bytes']=total_bytes;progress['last_at']=now
                            put('chunk',chunk)
                    if not stop.is_set():
                        transition(request_id,'upstream_body_complete',upstream_bytes=total_bytes)
                        put('done')
            except urllib.error.HTTPError as e:put('http_error',e.code)
            except Exception as e:put('error',type(e).__name__)
            finally:upstream_done.set()
        threading.Thread(target=upstream,daemon=True,name='proxy-upstream-'+request_id).start()
        usage=None;completed=False;client_alive=True;response_started=False;outcome='unknown'
        acc=bytearray();stream_line=bytearray();saw_done=False
        try:
            while True:
                remaining=deadline-time.monotonic()
                if remaining<=0:raise TimeoutError('upstream total-response deadline')
                try:kind,value=events.get(timeout=remaining)
                except queue.Empty:raise TimeoutError('upstream total-response deadline') from None
                if kind=='headers':
                    transition(request_id,'upstream_response',status=value[0])
                    try:
                        self.connection.settimeout(max(.001,deadline-time.monotonic()))
                        self.send_response(value[0]);self.send_header('Content-Type',value[1]);self.send_header('Transfer-Encoding','chunked');self.end_headers();response_started=True
                    except (OSError,ValueError) as e:
                        client_alive=False;transition(request_id,'client_disconnected',error=type(e).__name__)
                elif kind=='chunk':
                    if v.get('stream'):
                        stream_line.extend(value)
                        while b'\n' in stream_line:
                            line,_,rest=stream_line.partition(b'\n');stream_line=bytearray(rest)
                            if line.startswith(b'data: '):
                                if line[6:].strip()==b'[DONE]':saw_done=True
                                else:
                                    try:
                                        u=json.loads(line[6:]).get('usage')
                                        if u is not None:usage=u
                                    except (ValueError,UnicodeDecodeError):pass
                    else:acc.extend(value)
                    if client_alive:
                        try:
                            if os.environ.get('HS_PROXY_TEST_UPSTREAM')=='1' and os.environ.get('HS_PROXY_TEST_BLOCK_FORWARD_SECS'):
                                transition(request_id,'coordinator_forward_enter')
                                if os.environ.get('HS_PROXY_TEST_HOLD_COORDINATOR_LOCK')=='1':
                                    with LOCK:time.sleep(float(os.environ['HS_PROXY_TEST_BLOCK_FORWARD_SECS']))
                                else:time.sleep(float(os.environ['HS_PROXY_TEST_BLOCK_FORWARD_SECS']))
                            self.connection.settimeout(max(.001,deadline-time.monotonic()))
                            self.wfile.write(('%X\r\n'%len(value)).encode()+value+b'\r\n');self.wfile.flush()
                        except (OSError,ValueError) as e:
                            client_alive=False;transition(request_id,'client_disconnected',error=type(e).__name__)
                elif kind=='done':
                    if time.monotonic()>=deadline:raise TimeoutError('upstream total-response deadline')
                    if v.get('stream'):
                        if not saw_done or cost(usage) is None:raise ValueError('incomplete upstream SSE usage')
                    else:
                        usage=json.loads(acc).get('usage')
                    if client_alive:
                        self.connection.settimeout(max(.001,deadline-time.monotonic()))
                        self.wfile.write(b'0\r\n\r\n');self.wfile.flush()
                    if time.monotonic()>=deadline:raise TimeoutError('upstream total-response deadline')
                    usage_box['snapshot']=(usage,True)
                    completed=True;outcome='upstream_complete';break
                elif kind=='http_error':
                    outcome='upstream_http_error';transition(request_id,outcome,status=value)
                    if client_alive and not response_started:self.send_error(value,'provider error')
                    break
                elif kind=='error':
                    outcome='upstream_error';transition(request_id,outcome,error=value)
                    if client_alive and not response_started:self.send_error(502,'upstream or stream error')
                    break
        except Exception as e:
            outcome='upstream_error';transition(request_id,outcome,error=type(e).__name__)
            if client_alive and not response_started:
                try:self.send_error(502,'upstream or stream error')
                except OSError:pass
        finally:
            stop.set()
            # Never allow an incomplete chunked response to linger on keepalive.
            if response_started and not completed:self.close_connection=True
            settle_once(outcome)

class AdminHandler(BaseHTTPRequestHandler):
    def log_message(self,*args):pass
    def token_valid(self):
        return ADMIN_TOKEN is not None and hmac.compare_digest(self.path,'/load/'+ADMIN_TOKEN)
    def do_GET(self):
        if not self.token_valid() or MEMORY_KEY is not None:self.send_error(404);return
        form=b'<!doctype html><meta charset="utf-8"><title>Private key handoff</title><form method="post" autocomplete="off"><label>API key <input type="password" name="key" autocomplete="off" required></label><button type="submit">Load key</button></form>'
        self.send_response(200);self.send_header('Content-Type','text/html; charset=utf-8');self.send_header('Cache-Control','no-store');self.send_header('Referrer-Policy','no-referrer');self.send_header('Content-Security-Policy',"default-src 'none'; form-action 'self'; base-uri 'none'");self.send_header('Content-Length',str(len(form)));self.end_headers();self.wfile.write(form)
    def do_POST(self):
        global MEMORY_KEY,ADMIN_TOKEN
        if not self.token_valid() or MEMORY_KEY is not None:self.send_error(404);return
        length=self.headers.get('Content-Length','')
        if not length.isdigit() or int(length)>8192:self.send_error(413);return
        try:
            fields=urllib.parse.parse_qs(self.rfile.read(int(length)).decode('utf-8'),strict_parsing=True)
            values=fields['key'];assert len(values)==1 and values[0].strip()
        except (ValueError,KeyError,AssertionError,UnicodeDecodeError):self.send_error(400);return
        with KEY_LOCK:
            if MEMORY_KEY is not None:self.send_error(409);return
            MEMORY_KEY=values[0].strip();ADMIN_TOKEN=None
        done=b'Key loaded into proxy memory. You may close this page.'
        self.send_response(200);self.send_header('Content-Type','text/plain');self.send_header('Cache-Control','no-store');self.send_header('Content-Length',str(len(done)));self.end_headers();self.wfile.write(done)
        # One-time admin listener closes after successful POST; model proxy remains.
        threading.Thread(target=self.server.shutdown,daemon=True).start()

if __name__=='__main__':
    wall_start=time.time();mono_start=time.monotonic()
    time.sleep(.25)
    wall_delta=time.time()-wall_start;mono_delta=time.monotonic()-mono_start
    print(json.dumps({'port':A.port,'reserve_micros_per_call':RESERVE,'ceiling_micros':A.ceiling_micros}),flush=True)
    print(json.dumps({'event':'proxy_clock_calibration','wall_delta_secs':wall_delta,
        'monotonic_delta_secs':mono_delta,'wall_per_monotonic':wall_delta/mono_delta,
        'at':time.time(),'monotonic_at':time.monotonic()}),flush=True)
    if A.admin_port:threading.Thread(target=ThreadingHTTPServer(('127.0.0.1',A.admin_port),AdminHandler).serve_forever,daemon=True).start()
    ThreadingHTTPServer(('127.0.0.1',A.port),Handler).serve_forever()
