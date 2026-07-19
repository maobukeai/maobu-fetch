"""Deterministic HTTP Range server for LumaGet integration checks."""

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import argparse
import hashlib
import json
import threading


PAYLOAD = bytes(range(256)) * (128 * 1024)  # 32 MiB
REQUESTS: list[str] = []
LOCK = threading.Lock()


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
        if self.path == "/requests":
            with LOCK:
                body = json.dumps(REQUESTS).encode()
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
        self.wfile.write(body)

    def log_message(self, _format, *_args):
        return


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=18765)
    parser.add_argument("--info", action="store_true")
    args = parser.parse_args()
    if args.info:
        print(json.dumps({"bytes": len(PAYLOAD), "sha256": hashlib.sha256(PAYLOAD).hexdigest()}))
    ThreadingHTTPServer(("127.0.0.1", args.port), Handler).serve_forever()
