#!/usr/bin/env python3
"""Local upload sink for gate G3.

Routes:
  POST /worker-start   a service-worker spawn beacon; the reply says whether to run
  GET  /config         the run parameters
  PUT  /part/<n>       one multipart part; records arrival, size and a cheap digest
  POST /event          a free-form timeline entry from any extension context
  POST /result         the driver's final report; stores it and stops the server
"""

import http.server
import json
import os
import sys
import threading
import time

PORT = int(os.environ.get("G3_PORT", "8732"))
OUT = os.environ.get("G3_OUT", "/tmp/g3-result.json")
VARIANT = os.environ.get("G3_VARIANT", "offscreen")
PART_BYTES = int(os.environ.get("G3_PART_BYTES", str(5 * 1024 * 1024)))
TOTAL_BYTES = int(os.environ.get("G3_TOTAL_BYTES", str(256 * 1024 * 1024)))
PART_DELAY = float(os.environ.get("G3_PART_DELAY", "0.8"))
LABEL = os.environ.get("G3_LABEL", "run")

started = time.time()
lock = threading.Lock()
parts = []
events = []
worker_starts = []
run_claimed = threading.Event()
done = threading.Event()


def stamp():
    return round(time.time() - started, 3)


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        pass

    def _json(self, code, payload):
        body = json.dumps(payload).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _empty(self, code):
        self.send_response(code)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Content-Length", "0")
        self.end_headers()

    def _body(self):
        length = int(self.headers.get("Content-Length", "0"))
        remaining = length
        chunks = []
        while remaining > 0:
            block = self.rfile.read(min(remaining, 1 << 20))
            if not block:
                break
            chunks.append(block)
            remaining -= len(block)
        return b"".join(chunks)

    def do_OPTIONS(self):
        self._empty(204)

    def do_GET(self):
        if self.path == "/config":
            self._json(200, config())
            return
        self._empty(404)

    def do_PUT(self):
        if self.path.startswith("/part/"):
            index = self.path.rsplit("/", 1)[-1]
            raw = self._body()
            if PART_DELAY:
                time.sleep(PART_DELAY)
            with lock:
                parts.append(
                    {
                        "index": int(index),
                        "at": stamp(),
                        "bytes": len(raw),
                        "first": raw[0] if raw else None,
                        "last": raw[-1] if raw else None,
                    }
                )
            self._json(200, {"ok": True, "index": int(index), "bytes": len(raw)})
            return
        self._empty(404)

    def do_POST(self):
        if self.path == "/worker-start":
            raw = self._body()
            first = not run_claimed.is_set()
            run_claimed.set()
            with lock:
                worker_starts.append(
                    {"at": stamp(), "isFirst": first, "payload": json.loads(raw or b"{}")}
                )
            sys.stderr.write("worker spawn #%d at %.3fs\n" % (len(worker_starts), stamp()))
            self._json(200, {"start": first, "config": config()})
            return
        if self.path == "/event":
            raw = self._body()
            entry = json.loads(raw or b"{}")
            entry["at"] = stamp()
            with lock:
                events.append(entry)
            sys.stderr.write("event %s at %.3fs\n" % (entry.get("name"), entry["at"]))
            self._empty(204)
            return
        if self.path == "/result":
            raw = self._body()
            report = json.loads(raw or b"{}")
            with lock:
                report["server"] = {
                    "label": LABEL,
                    "variant": VARIANT,
                    "partDelaySeconds": PART_DELAY,
                    "workerStarts": worker_starts,
                    "events": events,
                    "partsReceived": len(parts),
                    "bytesReceived": sum(p["bytes"] for p in parts),
                    "firstPartAt": parts[0]["at"] if parts else None,
                    "lastPartAt": parts[-1]["at"] if parts else None,
                    "partIndexesInOrder": [p["index"] for p in parts],
                    "partSizes": sorted({p["bytes"] for p in parts}),
                    "partArrivals": [[p["index"], p["at"]] for p in parts],
                }
            with open(OUT, "w") as handle:
                json.dump(report, handle, indent=2)
            sys.stderr.write("result written to %s\n" % OUT)
            self._empty(204)
            done.set()
            return
        self._empty(404)


def config():
    return {
        "variant": VARIANT,
        "partBytes": PART_BYTES,
        "totalBytes": TOTAL_BYTES,
        "origin": "http://127.0.0.1:%d" % PORT,
        "label": LABEL,
    }


def main():
    server = http.server.ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    sys.stderr.write("sink listening on 127.0.0.1:%d variant=%s\n" % (PORT, VARIANT))
    if not done.wait(timeout=float(os.environ.get("G3_TIMEOUT", "600"))):
        sys.stderr.write("sink timed out; parts received: %d\n" % len(parts))
        with open(OUT, "w") as handle:
            json.dump(
                {
                    "timedOut": True,
                    "server": {
                        "label": LABEL,
                        "variant": VARIANT,
                        "workerStarts": worker_starts,
                        "events": events,
                        "partsReceived": len(parts),
                        "bytesReceived": sum(p["bytes"] for p in parts),
                    },
                },
                handle,
                indent=2,
            )
        server.shutdown()
        sys.exit(1)
    server.shutdown()


if __name__ == "__main__":
    main()
