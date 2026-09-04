#!/usr/bin/env python3
"""Local LLM egress relay: plugin -> http://127.0.0.1:PORT (plain) -> TLS upstream
with TCP keepalive 30/10/3. After a box suspend/resume the upstream socket gets
reset within seconds (proven 9:44), so the relay fails the request FAST (502)
instead of letting the caller hang on a dead socket. Harness loop treats the
error as provider feedback and retries - the mission survives the freeze."""
import http.server, socketserver, http.client, ssl, socket, os, json, time

TARGET_HOST = os.environ.get("RELAY_TARGET_HOST", "api.z.ai")
TARGET_PREFIX = os.environ.get("RELAY_TARGET_PREFIX", "/api/coding/paas/v4")
PORT = int(os.environ.get("RELAY_PORT", "8787"))

class KAHTTPSConnection(http.client.HTTPSConnection):
    def connect(self):
        super().connect()
        s = self.sock
        s.setsockopt(socket.SOL_SOCKET, socket.SO_KEEPALIVE, 1)
        s.setsockopt(socket.IPPROTO_TCP, socket.TCP_KEEPIDLE, 30)
        s.setsockopt(socket.IPPROTO_TCP, socket.TCP_KEEPINTVL, 10)
        s.setsockopt(socket.IPPROTO_TCP, socket.TCP_KEEPCNT, 3)

def rlog(msg):
    with open("/tmp/relay.log", "a") as f:
        f.write(f"{time.strftime('%F %T')} {msg}\n")

class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def do_POST(self):
        n = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(n)
        conn = KAHTTPSConnection(TARGET_HOST, 443, timeout=1500,
                                 context=ssl.create_default_context())
        try:
            fwd = {k: v for k, v in self.headers.items()
                   if k.lower() not in ("host", "content-length", "connection")}
            conn.request("POST", TARGET_PREFIX + self.path, body=body, headers=fwd)
            resp = conn.getresponse()
            data = resp.read()
            rlog(f"POST {self.path} -> {resp.status} ({len(data)} bytes)")
            self.send_response(resp.status)
            self.send_header("Content-Type", resp.getheader("Content-Type") or "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        except (OSError, http.client.HTTPException) as e:
            rlog(f"POST {self.path} -> 502 fast-fail: {e!r}")
            payload = json.dumps({"error": {"code": "relay_upstream_dead",
                "message": f"upstream dead (post-freeze fast fail): {e!r}"}}).encode()
            try:
                self.send_response(502)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)
            except OSError:
                pass
        finally:
            conn.close()
    def log_message(self, fmt, *args):
        pass

class ThreadingHTTPServer(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True
    allow_reuse_address = True

rlog(f"relay start :{PORT} -> https://{TARGET_HOST}{TARGET_PREFIX}")
ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
