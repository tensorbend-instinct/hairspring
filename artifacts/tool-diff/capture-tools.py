from http.server import HTTPServer,BaseHTTPRequestHandler
from pathlib import Path
class H(BaseHTTPRequestHandler):
 def log_message(self,*a):pass
 def do_POST(self):
  body=self.rfile.read(int(self.headers.get('Content-Length',0)))
  import json
  x=json.loads(body)
  if x.get('tools'): Path('/tmp/hs/artifacts/tool-diff/dsh-wire-request.json').write_bytes(body)
  self.send_response(401);self.send_header('Content-Type','application/json');self.end_headers();self.wfile.write(b'{"type":"error","error":{"type":"authentication_error","message":"local schema capture only"}}')
HTTPServer(('127.0.0.1',8766),H).serve_forever()
