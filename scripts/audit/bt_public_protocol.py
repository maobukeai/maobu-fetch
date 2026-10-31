"""Bounded public BT-v1 protocol probe; no torrent client, files or uploads."""
import hashlib
import ipaddress
import socket
import struct
import time

PAYLOAD_CAP = 2 * 1024 * 1024


class Bencode:
    def __init__(self, data):
        self.data, self.at, self.items, self.info_bytes = data, 0, 0, None

    def value(self, depth=0):
        self.items += 1
        if depth > 16 or self.items > 100000 or self.at >= len(self.data):
            raise ValueError("bencode bound exceeded")
        token = self.data[self.at:self.at + 1]
        if token == b"i":
            end = self.data.index(b"e", self.at)
            result = int(self.data[self.at + 1:end]); self.at = end + 1
            return result
        if token in (b"d", b"l"):
            self.at += 1
            result = {} if token == b"d" else []
            while self.data[self.at:self.at + 1] != b"e":
                key = self.value(depth + 1)
                if token == b"l":
                    result.append(key)
                else:
                    start = self.at
                    result[key] = self.value(depth + 1)
                    if depth == 0 and key == b"info":
                        self.info_bytes = self.data[start:self.at]
            self.at += 1
            return result
        colon = self.data.index(b":", self.at)
        length = int(self.data[self.at:colon]); self.at = colon + 1
        if length < 0 or self.at + length > len(self.data):
            raise ValueError("bencode string outside input")
        result = self.data[self.at:self.at + length]; self.at += length
        return result


def compact_peers(data):
    if not isinstance(data, bytes) or len(data) % 6:
        return []
    result = []
    for offset in range(0, len(data), 6):
        address = socket.inet_ntoa(data[offset:offset + 4])
        port = struct.unpack(">H", data[offset + 4:offset + 6])[0]
        if ipaddress.ip_address(address).is_global and port:
            result.append((address, port))
    return result


def remaining(deadline, maximum=3):
    result = min(maximum, deadline - time.monotonic())
    if result <= 0:
        raise TimeoutError("45-second probe deadline")
    return result


def receive(connection, length, deadline, counters):
    if length < 0 or counters["wire_bytes"] + length > PAYLOAD_CAP + 1024 * 1024:
        raise ValueError("control/wire read budget exceeded")
    data = bytearray()
    while len(data) < length:
        connection.settimeout(remaining(deadline))
        chunk = connection.recv(min(16384, length - len(data)))
        if not chunk:
            raise ConnectionError("peer closed connection")
        data.extend(chunk); counters["wire_bytes"] += len(chunk)
    return data


def frame(connection, deadline, counters):
    size = struct.unpack(">I", receive(connection, 4, deadline, counters))[0]
    if size == 0:
        return -1, b""
    if size > 512 * 1024:
        raise ValueError("peer frame exceeds control bound")
    kind = receive(connection, 1, deadline, counters)[0]
    if kind == 7:
        if size < 9 or size - 9 > 16384 or counters["payload_bytes"] + size - 9 > PAYLOAD_CAP:
            raise ValueError("peer payload exceeds requested block or remaining budget")
        body = receive(connection, size - 1, deadline, counters)
        counters["payload_bytes"] += size - 9
        return kind, body
    return kind, receive(connection, size - 1, deadline, counters)


def probe_peer(peer, info_hash, peer_id, info, deadline, counters):
    piece_length = info[b"piece length"]
    total = info[b"length"]
    count = (total + piece_length - 1) // piece_length
    if piece_length > PAYLOAD_CAP:
        return {"status": "partial", "reason": "piece larger than 2MiB; no complete hash validation claimed"}
    with socket.create_connection(peer, timeout=remaining(deadline)) as connection:
        connection.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 64 * 1024)
        connection.sendall(b"\x13BitTorrent protocol" + bytes(8) + info_hash + peer_id)
        handshake = receive(connection, 68, deadline, counters)
        if handshake[:20] != b"\x13BitTorrent protocol" or handshake[28:48] != info_hash:
            raise ValueError("peer handshake identity mismatch")
        counters["valid_handshakes"] += 1
        connection.sendall(struct.pack(">IB", 1, 2))  # Interested; never advertise/serve any data.
        unchoked, candidate = False, None
        for _ in range(256):
            kind, data = frame(connection, deadline, counters)
            if kind == 1:
                unchoked = True
            elif kind == 5:
                candidate = next((index * 8 + bit for index, value in enumerate(data[:(count + 7) // 8])
                    for bit in range(8) if value & (128 >> bit) and index * 8 + bit < count), candidate)
            elif kind == 4 and len(data) == 4:
                index = struct.unpack(">I", data)[0]
                if index < count: candidate = index if candidate is None else min(index, candidate)
            if unchoked and candidate is not None:
                break
        else:
            raise TimeoutError("no available unchoked piece")
        index = candidate
        if index is None:
            raise ValueError("peer has no valid piece index")
        length = min(piece_length, total - index * piece_length)
        if counters["requested_payload_bytes"] + length > PAYLOAD_CAP:
            raise ValueError("cumulative request budget exhausted")
        digest = hashlib.sha1()
        for offset in range(0, length, 16384):
            wanted = min(16384, length - offset)
            counters["requested_payload_bytes"] += wanted
            connection.sendall(struct.pack(">IBIII", 13, 6, index, offset, wanted))
            for _ in range(256):
                kind, data = frame(connection, deadline, counters)
                if kind == 7:
                    if struct.unpack(">II", data[:8]) != (index, offset) or len(data) - 8 != wanted:
                        raise ValueError("unexpected piece block response")
                    digest.update(memoryview(data)[8:]); break
                if kind == 0:
                    raise ConnectionError("peer choked requested transfer")
            else:
                raise TimeoutError("requested block not received")
        actual = digest.digest()
        if actual != info[b"pieces"][index * 20:(index + 1) * 20]:
            raise ValueError("public piece SHA1 mismatch")
        return {"status": "passed", "piece_index": index, "verified_payload_bytes": length, "piece_sha1": actual.hex()}
