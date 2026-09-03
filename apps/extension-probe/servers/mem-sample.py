#!/usr/bin/env python3
"""Samples RSS of every process whose /proc/<pid>/cmdline names a given marker.

Reads VmRSS from /proc/<pid>/status directly, which is the same number ps
reports; the marker is the browser's --user-data-dir, so only the instance
under test is sampled.
"""

import json
import os
import sys
import time

MARKER = sys.argv[1]
OUT = sys.argv[2]
INTERVAL = float(os.environ.get("MEM_INTERVAL", "0.5"))
DURATION = float(os.environ.get("MEM_DURATION", "900"))
STOP = os.environ.get("MEM_STOP", OUT + ".stop")


def cmdline(pid):
    try:
        with open("/proc/%s/cmdline" % pid, "rb") as handle:
            return handle.read().decode("utf-8", "replace").replace("\0", " ")
    except OSError:
        return None


def rss_kib(pid):
    try:
        with open("/proc/%s/status" % pid) as handle:
            for row in handle:
                if row.startswith("VmRSS:"):
                    return int(row.split()[1])
    except OSError:
        return None
    return None


def process_type(args):
    for token in args.split():
        if token.startswith("--type="):
            return token[len("--type=") :]
    return "browser"


def main():
    peaks = {}
    total_peak = 0
    samples = 0
    deadline = time.time() + DURATION
    while time.time() < deadline:
        total = 0
        for pid in os.listdir("/proc"):
            if not pid.isdigit():
                continue
            args = cmdline(pid)
            if not args or MARKER not in args:
                continue
            kib = rss_kib(pid)
            if kib is None:
                continue
            total += kib
            key = "%s:%s" % (pid, process_type(args))
            if kib > peaks.get(key, 0):
                peaks[key] = kib
        if total:
            samples += 1
            total_peak = max(total_peak, total)
        time.sleep(INTERVAL)
        if os.path.exists(STOP):
            break
    ranked = sorted(peaks.items(), key=lambda kv: -kv[1])
    with open(OUT, "w") as handle:
        json.dump(
            {
                "method": "/proc/<pid>/status VmRSS, sampled every %.1fs" % INTERVAL,
                "marker": MARKER,
                "samples": samples,
                "peakTotalRssMiB": round(total_peak / 1024, 1),
                "peakPerProcessRssMiB": [
                    {"process": key, "peakRssMiB": round(kib / 1024, 1)} for key, kib in ranked
                ],
            },
            handle,
            indent=2,
        )


if __name__ == "__main__":
    main()
