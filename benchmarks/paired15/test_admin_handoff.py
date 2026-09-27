import json,os,pathlib,socket,subprocess,tempfile,time,urllib.parse,urllib.request,urllib.error
ROOT=pathlib.Path(__file__).resolve().parent
def port():
 s=socket.socket();s.bind(('127.0.0.1',0));p=s.getsockname()[1];s.close();return p
with tempfile.TemporaryDirectory() as d:
 d=pathlib.Path(d);admin=port();model=port();token_file=d/'nonce';ledger=d/'ledger'
 proc=subprocess.Popen(['python3',str(ROOT/'spend_proxy.py'),'--admin-port',str(admin),'--admin-token-file',str(token_file),'--port',str(model),'--ledger',str(ledger),'--ceiling-micros','771860'],stdout=subprocess.PIPE,stderr=subprocess.PIPE)
 try:
  assert proc.stdout.readline()
  nonce=token_file.read_text();url=f'http://127.0.0.1:{admin}/load/{nonce}'
  for _ in range(100):
   try:page=urllib.request.urlopen(url,timeout=.2).read();break
   except OSError:time.sleep(.02)
  assert b'type="password"' in page and b'fake-test-key' not in page
  try:urllib.request.urlopen(f'http://127.0.0.1:{admin}/load/wrong-token',timeout=1)
  except urllib.error.HTTPError as e:assert e.code==404
  else:raise AssertionError('unguarded admin page')
  body=urllib.parse.urlencode({'key':'fake-test-key'}).encode();response=urllib.request.urlopen(url,data=body,timeout=2).read();assert b'Key loaded' in response
  for _ in range(100):
   try:urllib.request.urlopen(url,timeout=.1)
   except (OSError,urllib.error.HTTPError):break
   time.sleep(.02)
  else:raise AssertionError('admin listener remained open')
  assert not (d/'key').exists()
  print('nonce-protected password field; one-time browser POST loaded dummy key into memory; admin listener closed, no key file')
 finally:proc.terminate();proc.wait(timeout=3)
