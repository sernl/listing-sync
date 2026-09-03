#!/usr/bin/env python3
"""Local origin for gate G2: answers 302 with a Location, collects the report.

Routes:
  GET  /redirect-source   302 -> /redirect-target?product=<id>
  GET  /redirect-target   200
  POST /result            stores the extension's JSON report and stops the server
"""

import http.server
import json
import os
import sys
import threading

PORT = int(os.environ.get("G2_PORT", "8731"))
OUT = os.environ.get("G2_OUT", "/tmp/g2-result.json")
PRODUCT_ID = "88117001"

done = threading.Event()


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        sys.stderr.write("server %s %s\n" % (self.address_string(), fmt % args))

    def _cors(self):
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Headers", "content-type")
        self.send_header("Access-Control-Allow-Methods", "GET,POST,OPTIONS")

    def do_OPTIONS(self):
        self.send_response(204)
        self._cors()
        self.send_header("Content-Length", "0")
        self.end_headers()

    def do_GET(self):
        if self.path.startswith("/redirect-source"):
            self.send_response(302)
            self.send_header("Location", "/redirect-target?product=%s" % PRODUCT_ID)
            self.send_header("X-G2-Marker", "source-302")
            self._cors()
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        if self.path.startswith("/redirect-target"):
            body = b"landed"
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("X-G2-Marker", "target-200")
            self._cors()
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        self.send_response(404)
        self._cors()
        self.send_header("Content-Length", "0")
        self.end_headers()

    def do_POST(self):
        if self.path == "/result":
            length = int(self.headers.get("Content-Length", "0"))
            raw = self.rfile.read(length)
            with open(OUT, "wb") as handle:
                handle.write(raw)
            sys.stderr.write("server wrote %d bytes to %s\n" % (len(raw), OUT))
            self.send_response(204)
            self._cors()
            self.send_header("Content-Length", "0")
            self.end_headers()
            done.set()
            return
        self.send_response(404)
        self._cors()
        self.send_header("Content-Length", "0")
        self.end_headers()


def main():
    server = http.server.ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    sys.stderr.write("server listening on 127.0.0.1:%d\n" % PORT)
    if not done.wait(timeout=float(os.environ.get("G2_TIMEOUT", "90"))):
        sys.stderr.write("server timed out with no report\n")
        server.shutdown()
        sys.exit(1)
    server.shutdown()
    sys.stderr.write("server done\n")


if __name__ == "__main__":
    main()
