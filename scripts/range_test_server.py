"""Deterministic HTTP Range server for Maobu Fetch integration checks."""

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import argparse
import hashlib
import json
import threading
import time


PAYLOAD = bytes(range(256)) * (128 * 1024)  # 32 MiB
REQUESTS: list[str] = []
LOCK = threading.Lock()
ACTIVE = 0
PEAK_ACTIVE = 0
DROP_ONCE = False
DROPPED = False


class RangeServer(ThreadingHTTPServer):
    request_queue_size = 128
    daemon_threads = True


class Handler(BaseHTTPRequestHandler):
    def do_HEAD(self):
        if self.path != "/fixture.bin":
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Length", str(len(PAYLOAD)))
        self.send_header("ETag", '"lumaget-range-fixture-v1"')
        self.end_headers()

    def do_GET(self):
        global ACTIVE, PEAK_ACTIVE, DROPPED
        if self.path == "/requests":
            with LOCK:
                body = json.dumps(REQUESTS).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if self.path == "/metrics":
            with LOCK:
                body = json.dumps(
                    {"requests": REQUESTS, "peak_active": PEAK_ACTIVE, "dropped_once": DROPPED}
                ).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if self.path != "/fixture.bin":
            self.send_error(404)
            return
        value = self.headers.get("Range")
        if not value or not value.startswith("bytes="):
            start, end, status = 0, len(PAYLOAD) - 1, 200
        else:
            start_text, end_text = value[6:].split("-", 1)
            start = int(start_text)
            end = min(int(end_text) if end_text else len(PAYLOAD) - 1, len(PAYLOAD) - 1)
            status = 206
            with LOCK:
                REQUESTS.append(value)
        body = PAYLOAD[start : end + 1]
        self.send_response(status)
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("ETag", '"lumaget-range-fixture-v1"')
        if status == 206:
            self.send_header("Content-Range", f"bytes {start}-{end}/{len(PAYLOAD)}")
        self.end_headers()
        with LOCK:
            ACTIVE += 1
            PEAK_ACTIVE = max(PEAK_ACTIVE, ACTIVE)
        try:
            if DROP_ONCE and status == 206 and len(body) > 1 and not DROPPED:
                with LOCK:
                    should_drop = not DROPPED
                    DROPPED = True
                if should_drop:
                    self.wfile.write(body[: len(body) // 2])
                    self.wfile.flush()
                    self.close_connection = True
                    return
            for offset in range(0, len(body), 64 * 1024):
                self.wfile.write(body[offset : offset + 64 * 1024])
                self.wfile.flush()
                time.sleep(0.005)
        finally:
            with LOCK:
                ACTIVE -= 1

    def log_message(self, _format, *_args):
        return


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=18765)
    parser.add_argument("--info", action="store_true")
    parser.add_argument("--drop-once", action="store_true")
    parser.add_argument("--size-mib", type=int, default=32)
    args = parser.parse_args()
    PAYLOAD = bytes(range(256)) * (args.size_mib * 4096)
    DROP_ONCE = args.drop_once
    if args.info:
        print(json.dumps({"bytes": len(PAYLOAD), "sha256": hashlib.sha256(PAYLOAD).hexdigest()}))
    RangeServer(("127.0.0.1", args.port), Handler).serve_forever()
