"""Opt-in official Ubuntu metadata + bounded public protocol reachability probe.

No desktop client/session is started. This is not a production-engine public
download test. The caller must supply the freshly built isolated test EXE.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request

from bt_public_protocol import Bencode, PAYLOAD_CAP, compact_peers, probe_peer, remaining

SOURCE = "https://releases.ubuntu.com/24.04/ubuntu-24.04.5-live-server-amd64.iso.torrent"
TRACKER_HOSTS = {"torrent.ubuntu.com", "ipv6.torrent.ubuntu.com"}
TEST = "bt::public_probe_tests::inspect_downloaded_official_torrent_metadata"


class OfficialRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, new_url):
        if urllib.parse.urlparse(new_url).hostname not in TRACKER_HOSTS | {"releases.ubuntu.com"}:
            raise ValueError("non-official HTTP redirect refused")
        return super().redirect_request(request, fp, code, message, headers, new_url)


def get(url, deadline, limit):
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), OfficialRedirects())
    request = urllib.request.Request(url, headers={"User-Agent": "Maobu-Audit/1.0"})
    with opener.open(request, timeout=remaining(deadline, 10)) as response:
        data = bytearray()
        while True:
            remaining(deadline)
            chunk = response.read1(min(16384, limit + 1 - len(data)))
            if not chunk: return bytes(data)
            data.extend(chunk)
            if len(data) > limit: raise ValueError("HTTP metadata size cap exceeded")


def production_inspection(exe, path, sha256, deadline):
    environment = os.environ.copy()
    environment["MAOBU_PUBLIC_TORRENT_PATH"] = str(path)
    environment["MAOBU_PUBLIC_TORRENT_SHA256"] = sha256
    process = subprocess.run([str(exe), "--exact", TEST, "--ignored", "--nocapture"], env=environment,
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace", timeout=remaining(deadline, 12))
    marker = "UBUNTU_PRODUCTION_METADATA "
    line = next((line for line in process.stdout.splitlines() if marker in line), None)
    if process.returncode or not line: raise RuntimeError("production metadata validation test did not execute/pass")
    return json.loads(line.split(marker, 1)[1])


def announce_url(tracker, info_hash, peer_id, port, total, event="started", downloaded=0):
    quote = urllib.parse.quote_from_bytes
    return tracker + "?info_hash=" + quote(info_hash, safe="") + "&peer_id=" + quote(peer_id, safe="") + "&" + urllib.parse.urlencode({
        "port": port, "uploaded": 0, "downloaded": downloaded, "left": max(0, total - downloaded),
        "compact": 1, "numwant": 20, "event": event}, doseq=True)


def public_protocol(metadata, info_hash, deadline, result):
    info = metadata[b"info"]
    trackers = metadata.get(b"announce-list", [])
    trackers = [value for group in trackers for value in group] + [metadata.get(b"announce", b"")]
    trackers = list(dict.fromkeys(value.decode("utf-8") for value in trackers if value))
    trackers = [value for value in trackers if urllib.parse.urlparse(value).hostname in TRACKER_HOSTS
        and urllib.parse.urlparse(value).scheme in ("http", "https")]
    counters = {"wire_bytes": 0, "payload_bytes": 0, "requested_payload_bytes": 0, "valid_handshakes": 0}
    result["counters"] = counters
    result["peer_attempts"] = []
    peer_id = b"-MB1000-" + os.urandom(12)
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))  # reserve an unused local port; never listen
        port = reservation.getsockname()[1]
        peers, active_tracker = [], None
        result["trackers"] = []
        for tracker in trackers[:2]:
            started = time.monotonic()
            try:
                data = get(announce_url(tracker, info_hash, peer_id, port, info[b"length"]), deadline, 256 * 1024)
                response = Bencode(data).value()
                if response.get(b"failure reason"): raise RuntimeError("official tracker rejected announce")
                peers = compact_peers(response.get(b"peers", b"")); active_tracker = tracker
                result["trackers"].append({"url": tracker, "status": "passed", "public_ipv4_peers": len(peers), "duration_ms": round((time.monotonic() - started) * 1000)})
                if peers: break
            except Exception as error:
                result["trackers"].append({"url": tracker, "status": "unreachable", "reason": type(error).__name__})
        try:
            for peer in peers[:3]:
                try:
                    attempt = probe_peer(peer, info_hash, peer_id, info, deadline, counters)
                    result["peer_attempts"].append(attempt)
                    if attempt["status"] == "passed": result["public_protocol"] = "passed"; break
                except Exception as error:
                    result["peer_attempts"].append({"status": "failed", "reason": type(error).__name__})
        finally:
            if active_tracker and deadline > time.monotonic():
                try:
                    get(announce_url(active_tracker, info_hash, peer_id, port, info[b"length"], "stopped", counters["payload_bytes"]), deadline, 256 * 1024)
                    result["stopped_announce"] = "sent"
                except Exception: result["stopped_announce"] = "not_confirmed"
    assert counters["payload_bytes"] <= PAYLOAD_CAP and counters["requested_payload_bytes"] <= PAYLOAD_CAP


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-exe", required=True, type=Path)
    parser.add_argument("--metadata-path", type=Path)
    args = parser.parse_args()
    expected_executables = Path(__file__).resolve().parents[2] / "src-tauri/target/debug/deps"
    executable = args.test_exe.resolve()
    if executable.parent != expected_executables.resolve() or not executable.name.startswith("maobu_fetch_lib-") or executable.suffix != ".exe":
        raise SystemExit("Only the workspace's freshly built isolated Cargo test EXE may be launched")
    root = args.metadata_path.parent if args.metadata_path else Path(tempfile.mkdtemp(prefix="maobu-public-bt-"))
    if not root.resolve().is_relative_to(Path(tempfile.gettempdir()).resolve()) or not root.name.startswith("maobu-public-bt-"):
        raise SystemExit("Probe output must stay in its dedicated system temporary directory")
    deadline = time.monotonic() + 45
    result = {"source": SOURCE, "observed_at_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "layer": "public network/protocol reachability; not production-engine public download",
        "production_metadata": "pending", "public_protocol": "unverified", "payload_cap_bytes": PAYLOAD_CAP}
    destination = root / "public-probe-result.json"
    def deadline_exit():
        result["probe_failure"] = "hard 45-second deadline"
        result["duration_ms"] = 45000
        destination.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
        print(json.dumps({"report": str(destination), **result}, ensure_ascii=False, indent=2), flush=True)
        os._exit(2)  # only this standalone probe process; OS closes its outbound sockets.
    watchdog = threading.Timer(45, deadline_exit)
    watchdog.daemon = True
    watchdog.start()
    try:
        path = args.metadata_path or root / SOURCE.rsplit("/", 1)[1]
        data = path.read_bytes() if args.metadata_path else get(SOURCE, deadline, 1024 * 1024)
        if len(data) > 1024 * 1024: raise ValueError("metadata over 1MiB")
        if not args.metadata_path: path.write_bytes(data)
        else:
            provenance = json.loads((root / "download-result.json").read_text(encoding="utf-8"))
            if provenance.get("source") != SOURCE or provenance.get("sha256") != hashlib.sha256(data).hexdigest() or provenance.get("metadata_http") != "passed":
                raise ValueError("cached metadata lacks matching official HTTPS provenance")
        result.update(metadata_bytes=len(data), metadata_sha256=hashlib.sha256(data).hexdigest())
        decoded = Bencode(data); metadata = decoded.value()
        digest = hashlib.sha1(decoded.info_bytes).digest()
        inspected = production_inspection(executable, path, result["metadata_sha256"], deadline)
        if inspected["info_hash"] != digest.hex(): raise ValueError("production infohash mismatch")
        result.update(production_metadata="passed", info_hash=digest.hex(), piece_length=metadata[b"info"][b"piece length"], name=inspected["name"])
        print(json.dumps({"phase": "production_metadata_passed", "test_exe_released": True, "info_hash": digest.hex()}), flush=True)
        public_protocol(metadata, digest, deadline, result)
    except Exception as error:
        result["probe_failure"] = type(error).__name__ + ": " + str(error)
    watchdog.cancel()
    result["duration_ms"] = round((45 - max(0, deadline - time.monotonic())) * 1000)
    destination.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps({"report": str(destination), **result}, ensure_ascii=False, indent=2))
    return 0 if result["public_protocol"] == "passed" else 2


if __name__ == "__main__": raise SystemExit(main())
