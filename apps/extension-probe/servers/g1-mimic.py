#!/usr/bin/env python3
"""Local stand-in for the three page shapes gate G1 must tell apart.

Serves a login-like page with a password input, a Cloudflare-like interstitial,
a redirect, and a create-form-like page carrying the five CakePHP hidden inputs
and the credentials block that `crates/tam-marketplace-tpt/src/form.rs` scrapes.
Nothing here contacts a marketplace; the markers are literal strings.
"""

import http.server
import json
import os
import sys
import threading

PORT = int(os.environ.get("G1_PORT", "8733"))
OUT = os.environ.get("G1_OUT", "/tmp/g1-selfcheck.json")

done = threading.Event()

LANDING = b"""<!doctype html><html><head><title>Mimic landing</title></head>
<body><h1>mimic origin</h1><p>An ordinary page with no markers.</p></body></html>"""

LOGIN = b"""<!doctype html><html><head><title>Sign in</title></head>
<body><form method="post"><input name="user" type="text">
<input name="pass" type="password"><button>Sign in</button></form></body></html>"""

CHALLENGE = b"""<!doctype html><html><head><title>Just a moment...</title></head>
<body><h1>Checking your browser</h1>
<script src="/cdn-cgi/challenge-platform/h/b/orchestrate/chl_page/v1"></script>
<div class="cf-browser-verification"></div></body></html>"""

CREATE_FORM = b"""<!doctype html><html><head><title>New digital product</title></head>
<body><form id="ProductForm" method="post" enctype="multipart/form-data">
<input type="hidden" name="data[_Token][key]" value="MIMICKEY">
<input type="hidden" name="data[_Token][fields]" value="MIMICFIELDS">
<input type="hidden" name="data[_Token][unlocked]" value="MIMICUNLOCKED">
<input type="hidden" name="data[_Csrf][csrfKey]" value="MIMICCSRFKEY">
<input type="hidden" name="data[_Csrf][csrfToken]" value="MIMICCSRFTOKEN">
<input name="data[Product][title]" type="text">
</form>
<script>var uploadConfig = {"credentials":{"key":"MIMICAWSKEY","policy":"x"}};</script>
</body></html>"""


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        sys.stderr.write("mimic %s\n" % (fmt % args))

    def _send(self, code, body, extra=None):
        self.send_response(code)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("CF-RAY", "7a1b2c3d4e5f6789-LHR")
        self.send_header("CF-Cache-Status", "DYNAMIC")
        self.send_header("Set-Cookie", "csrfToken=mimic-csrf-value; Path=/")
        self.send_header("Set-Cookie", "session_id=mimic-session; Path=/; HttpOnly")
        for name, value in (extra or []):
            self.send_header(name, value)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def do_GET(self):
        path = self.path.split("?")[0]
        if path == "/Login":
            self._send(200, LOGIN)
        elif path == "/challenge":
            self._send(403, CHALLENGE)
        elif path == "/redirect":
            self._send(302, b"", [("Location", "/My-Products/New/Digital-Next")])
        elif path == "/My-Products/New/Digital-Next":
            self._send(200, CREATE_FORM)
        elif path == "/":
            self._send(200, LANDING)
        else:
            self._send(404, b"<html><title>not found</title></html>")

    def do_POST(self):
        if self.path == "/selfcheck":
            length = int(self.headers.get("Content-Length", "0"))
            raw = self.rfile.read(length)
            with open(OUT, "wb") as handle:
                handle.write(raw)
            sys.stderr.write("selfcheck written to %s (%d bytes)\n" % (OUT, len(raw)))
            self.send_response(204)
            self.send_header("Access-Control-Allow-Origin", "*")
            self.send_header("Content-Length", "0")
            self.end_headers()
            done.set()
            return
        self._send(404, b"")


def main():
    server = http.server.ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    sys.stderr.write("mimic listening on 127.0.0.1:%d\n" % PORT)
    if not done.wait(timeout=float(os.environ.get("G1_TIMEOUT", "120"))):
        sys.stderr.write("mimic timed out with no selfcheck\n")
        server.shutdown()
        sys.exit(1)
    server.shutdown()


if __name__ == "__main__":
    main()
