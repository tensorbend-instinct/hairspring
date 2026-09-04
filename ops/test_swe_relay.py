"""Seam tests for ops/swe-relay.py against a fake local upstream.

Covers the two production bites:
1. gzip encoding: the relay must strip Accept-Encoding before forwarding
   (ureq advertises gzip but cannot decode), and must forward an upstream
   Content-Encoding back to the caller if one arrives anyway.
2. post-freeze fast fail: dead upstream must produce a fast 502 with the
   relay_upstream_dead error code, not a hung socket.
"""
import gzip, http.server, json, os, socket, socketserver, subprocess, sys, threading, time, urllib.request, urllib.error

REPO = os.path.dirname(os.path.abspath(__file__))
RELAY = os.path.join(REPO, "swe-relay.py")

class Quiet(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a): pass

def free_port():
    s = socket.socket(); s.bind(("127.0.0.1", 0)); p = s.getsockname()[1]; s.close(); return p

def start_relay(port, env_extra):
    env = dict(os.environ, RELAY_PORT=str(port), **env_extra)
    p = subprocess.Popen([sys.executable, RELAY], env=env,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(50):
        try:
            socket.create_connection(("127.0.0.1", port), timeout=0.2).close(); break
        except OSError:
            time.sleep(0.1)
    else:
        p.kill(); raise RuntimeError("relay did not start")
    return p

def post(port, path="/x", body=b"{}", headers=None):
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", data=body,
                                 headers=headers or {}, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=10) as r:
            return r.status, dict(r.headers), r.read()
    except urllib.error.HTTPError as e:
        return e.code, dict(e.headers), e.read()

def test_accept_encoding_stripped_upstream():
    """The two-layer bug: caller sends Accept-Encoding: gzip; if that header
    reached the upstream, upstream would gzip and the caller could not decode.
    Relay must strip it (http.client may add an explicit "identity", which is safe). Also: if an upstream sends gzip anyway, the
    Content-Encoding header must survive the relay so the caller knows."""
    seen = {}
    class Up(Quiet):
        def do_POST(self):
            n = int(self.headers.get("Content-Length", 0))
            self.rfile.read(n)
            seen["ae"] = self.headers.get("Accept-Encoding")
            payload = gzip.compress(b'{"ok":true}')
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Encoding", "gzip")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
    class S(socketserver.ThreadingMixIn, http.server.HTTPServer): daemon_threads = True
    uport = free_port()
    srv = S(("127.0.0.1", uport), Up)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    rport = free_port()
    # test-only knobs: plain-http upstream on a chosen port
    relay = start_relay(rport, {"RELAY_UPSTREAM_PORT": str(uport),
                                "RELAY_UPSTREAM_SCHEME": "http",
                                "RELAY_TARGET_HOST": "127.0.0.1",
                                "RELAY_TARGET_PREFIX": ""})
    try:
        status, hdrs, body = post(rport, headers={"Accept-Encoding": "gzip", "Content-Type": "application/json"})
        ae = seen.get("ae")
        assert ae is None or "gzip" not in ae, f"gzip Accept-Encoding leaked upstream: {ae!r}"
        assert status == 200
        assert hdrs.get("Content-Encoding") == "gzip", "Content-Encoding not forwarded"
        assert gzip.decompress(body) == b'{"ok":true}'
    finally:
        relay.kill(); srv.shutdown()

def test_dead_upstream_fast_502():
    dead = free_port()  # nothing listening
    rport = free_port()
    relay = start_relay(rport, {"RELAY_UPSTREAM_PORT": str(dead),
                                "RELAY_UPSTREAM_SCHEME": "http",
                                "RELAY_TARGET_HOST": "127.0.0.1",
                                "RELAY_TARGET_PREFIX": ""})
    try:
        t0 = time.time()
        status, _, body = post(rport)
        dt = time.time() - t0
        assert status == 502, f"expected fast 502, got {status}"
        assert dt < 10, f"fast-fail took {dt:.1f}s"
        assert json.loads(body)["error"]["code"] == "relay_upstream_dead"
    finally:
        relay.kill()

def test_client_receives_502_when_upstream_drops_mid_call():
    """Regression for the 12:58 anomaly: upstream accepted the request then
    dropped the connection without responding (RemoteDisconnected). The
    relay must still deliver a well-framed 502 the client can consume -
    not leave the caller parked on an established-but-silent socket."""
    import http.client as hc
    class Drop(Quiet):
        def do_POST(self):
            n = int(self.headers.get("Content-Length", 0))
            self.rfile.read(n)
            self.connection.shutdown(socket.SHUT_RDWR)
            self.connection.close()
    class S(socketserver.ThreadingMixIn, http.server.HTTPServer): daemon_threads = True
    uport = free_port()
    srv = S(("127.0.0.1", uport), Drop)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    rport = free_port()
    relay = start_relay(rport, {"RELAY_UPSTREAM_PORT": str(uport),
                                "RELAY_UPSTREAM_SCHEME": "http",
                                "RELAY_TARGET_HOST": "127.0.0.1",
                                "RELAY_TARGET_PREFIX": ""})
    try:
        status, hdrs, body = post(rport)
        assert status == 502, f"caller saw {status}, not the fast-fail 502"
        assert json.loads(body)["error"]["code"] == "relay_upstream_dead"
        # framing must be complete: exact Content-Length bytes arrived
        assert hdrs.get("Content-Length") == str(len(body))
    finally:
        relay.kill(); srv.shutdown()
