"""Deterministic HTTP Range server for Maobu Fetch integration checks."""

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import argparse
import hashlib
import json
import threading
import time


PATTERN = bytes(range(256))
PAYLOAD_SIZE = 32 * 1024 * 1024
REQUESTS: list[str] = []
LOCK = threading.Lock()
ACTIVE = 0
PEAK_ACTIVE = 0
DROP_ONCE = False
DROPPED = False


def payload_bytes(start: int, length: int) -> bytes:
    """Generate the deterministic byte pattern without allocating the full fixture."""
    if length <= 0:
        return b""
    phase = start % len(PATTERN)
    rotated = PATTERN[phase:] + PATTERN[:phase]
    return (rotated * ((length + len(PATTERN) - 1) // len(PATTERN)))[:length]


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
        self.send_header("Content-Length", str(PAYLOAD_SIZE))
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
            start, end, status = 0, PAYLOAD_SIZE - 1, 200
        else:
            start_text, end_text = value[6:].split("-", 1)
            start = int(start_text)
            end = min(int(end_text) if end_text else PAYLOAD_SIZE - 1, PAYLOAD_SIZE - 1)
            status = 206
            with LOCK:
                REQUESTS.append(value)
        body_length = end - start + 1
        self.send_response(status)
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Length", str(body_length))
        self.send_header("ETag", '"lumaget-range-fixture-v1"')
        if status == 206:
            self.send_header("Content-Range", f"bytes {start}-{end}/{PAYLOAD_SIZE}")
        self.end_headers()
        with LOCK:
            ACTIVE += 1
            PEAK_ACTIVE = max(PEAK_ACTIVE, ACTIVE)
        try:
            send_length = body_length
            if DROP_ONCE and status == 206 and body_length > 1 and not DROPPED:
                with LOCK:
                    should_drop = not DROPPED
                    DROPPED = True
                if should_drop:
                    send_length = body_length // 2
            for offset in range(0, send_length, 64 * 1024):
                chunk_length = min(64 * 1024, send_length - offset)
                self.wfile.write(payload_bytes(start + offset, chunk_length))
                self.wfile.flush()
                if offset + chunk_length < send_length:
                    time.sleep(0.005)
            if send_length < body_length:
                self.close_connection = True
                return
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
    PAYLOAD_SIZE = args.size_mib * 1024 * 1024
    DROP_ONCE = args.drop_once
    if args.info:
        digest = hashlib.sha256()
        for offset in range(0, PAYLOAD_SIZE, 1024 * 1024):
            digest.update(payload_bytes(offset, min(1024 * 1024, PAYLOAD_SIZE - offset)))
        print(json.dumps({"bytes": PAYLOAD_SIZE, "sha256": digest.hexdigest()}))
    RangeServer(("127.0.0.1", args.port), Handler).serve_forever()
