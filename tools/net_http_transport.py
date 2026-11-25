"""Isolated E1000 HTTP/1.1 + DNS peer verifying redirects, keep-alive, chunked, gzip, and bounds."""
from __future__ import annotations

import gzip
import socket
import struct
from threading import Event

from net_dhcp_transport import ARP, BROADCAST_MAC, IPV4, ipv4
from net_dns_transport import DNS_PORT, host_udp, parse_tcp, parse_udp
from net_live_transport import ACK, FIN, PSH, SYN, advance, dns_answer, pseudo
from net_static_transport import arp, ethernet
from net_test_transport import EthernetPeer, GUEST_MAC, HOST_MAC, checksum

GUEST = bytes((192, 0, 2, 12))
DNS_HOST = bytes((192, 0, 2, 1))
HTTP_HOST = bytes((192, 0, 2, 2))
CONFLICT_MAC = bytes.fromhex("020000000077")
HTTP_PORT = 80
MSS = 1460
RST = 0x04

Q_HTTP = b"\x04http\x02du\x03net\x04test\x00\x00\x01\x00\x01"
GZIP_DATA = b"dunit-os HTTP/1.1 keep-alive compressed! " * 8
GZIP_BOMB = b"B" * 256


def host_tcp_http(dst_port: int, seq: int, ack: int, flags: int, data: bytes = b"") -> bytes:
    opts = struct.pack("!BBH", 2, 4, MSS) if (flags & SYN) else b""
    off = (5 + len(opts) // 4) << 4
    hdr = struct.pack("!HHIIBBHHH", HTTP_PORT, dst_port, seq, ack, off, flags, 16384, 0, 0) + opts
    seg = hdr + data
    chk = checksum(pseudo(HTTP_HOST, GUEST, 6, len(seg)) + seg)
    ip = ipv4(6, HTTP_HOST, GUEST, seg[:16] + struct.pack("!H", chk) + seg[18:])
    return ethernet(GUEST_MAC, HOST_MAC, IPV4, ip)


class NetHttpPeer(EthernetPeer):
    """Serves HTTP/1.1 scenarios over E1000 TCP + DNS and verifies each milestone."""

    def __init__(self, workdir):
        super().__init__(workdir)
        self._done = Event()
        self._stages: set[str] = set()

    def wait_complete(self, timeout: float) -> bool:
        expected = {
            "dns_resolved",
            "redirect_sent",
            "chunked_sent",
            "gzip_sent",
            "post_sent",
            "loop_hit",
            "bomb_sent",
            "truncated_sent",
            "conflict_injected",
        }
        return self._done.wait(timeout) and expected.issubset(self._stages)

    def _exchange(self, connection: socket.socket) -> None:
        try:
            self._serve(connection)
        finally:
            self._done.set()
        self._stop.wait()

    def _serve(self, connection: socket.socket) -> None:
        next_host_isn = 0x70000001
        conns: dict[int, dict] = {}

        while True:
            frame = self._receive(connection)
            eth = frame[12:14]
            if eth == ARP:
                if frame == ethernet(BROADCAST_MAC, GUEST_MAC, ARP, arp(1, GUEST_MAC, bytes(4), bytes(6), GUEST)):
                    continue
                if frame == ethernet(BROADCAST_MAC, GUEST_MAC, ARP, arp(1, GUEST_MAC, GUEST, bytes(6), GUEST)):
                    continue
                for target in (DNS_HOST, HTTP_HOST):
                    if frame == ethernet(BROADCAST_MAC, GUEST_MAC, ARP, arp(1, GUEST_MAC, GUEST, bytes(6), target)):
                        self._send(connection, ethernet(GUEST_MAC, HOST_MAC, ARP, arp(2, HOST_MAC, target, GUEST_MAC, GUEST)))
                        break
                continue

            if eth != IPV4:
                continue

            proto = frame[23]
            if proto == 17:
                dst, sport, dport, dns_bytes = parse_udp(frame)
                if dst != DNS_HOST or dport != DNS_PORT:
                    raise ValueError(f"unexpected UDP {dst!r}:{dport}")
                self._send(connection, host_udp(DNS_HOST, DNS_PORT, sport, dns_answer(dns_bytes, Q_HTTP, 2, False)))
                self._stages.add("dns_resolved")
                continue

            if proto != 6:
                continue

            dst, sport, dport, seq, ack, flags, data = parse_tcp(frame)
            if dst != HTTP_HOST or dport != HTTP_PORT:
                raise ValueError(f"unexpected TCP {dst!r}:{dport}")
            if flags & RST:
                conns.pop(sport, None)
                continue

            if flags & SYN:
                isn = next_host_isn
                next_host_isn = advance(next_host_isn, 0x10000)
                conn = {
                    "recv_next": advance(seq, 1),
                    "host_seq": isn,
                    "buf": bytearray(),
                    "fin_sent": False,
                }
                conns[sport] = conn
                self._send(connection, host_tcp_http(sport, isn - 1, conn["recv_next"], SYN | ACK))
                continue

            conn = conns.get(sport)
            if conn is None:
                continue

            if data and seq == conn["recv_next"]:
                conn["recv_next"] = advance(conn["recv_next"], len(data))
                conn["buf"].extend(data)
                self._send(connection, host_tcp_http(sport, conn["host_seq"], conn["recv_next"], ACK))

                while True:
                    parsed = self._pop_http_request(conn["buf"])
                    if parsed is None:
                        break
                    method, path, body = parsed
                    if path == "/after-conflict":
                        self._send(
                            connection,
                            ethernet(BROADCAST_MAC, CONFLICT_MAC, ARP, arp(1, CONFLICT_MAC, GUEST, bytes(6), GUEST)),
                        )
                        self._stages.add("conflict_injected")
                        return
                    reply_bytes, close_conn = self._handle_request(method, path, body)
                    t_flags = PSH | ACK | (FIN if close_conn else 0)
                    conn["fin_sent"] |= close_conn
                    self._send(connection, host_tcp_http(sport, conn["host_seq"], conn["recv_next"], t_flags, reply_bytes))
                    conn["host_seq"] = advance(conn["host_seq"], len(reply_bytes) + (1 if close_conn else 0))

            if flags & FIN:
                conn["recv_next"] = advance(seq, len(data) + 1)
                if conn["fin_sent"]:
                    self._send(connection, host_tcp_http(sport, conn["host_seq"], conn["recv_next"], ACK))
                else:
                    conn["fin_sent"] = True
                    self._send(connection, host_tcp_http(sport, conn["host_seq"], conn["recv_next"], FIN | ACK))
                    conn["host_seq"] = advance(conn["host_seq"], 1)

    def _pop_http_request(self, buf: bytearray) -> tuple[str, str, bytes] | None:
        end = buf.find(b"\r\n\r\n")
        if end < 0:
            return None
        head = buf[:end].decode("utf-8")
        lines = head.split("\r\n")
        parts = lines[0].split(" ")
        method, path = parts[0], parts[1]
        cl = 0
        for h in lines[1:]:
            k, _, v = h.partition(":")
            if k.strip().lower() == "content-length":
                cl = int(v.strip())
        total = end + 4 + cl
        if len(buf) < total:
            return None
        body = bytes(buf[end + 4:total])
        del buf[:total]
        return method, path, body

    def _handle_request(self, method: str, path: str, body: bytes) -> tuple[bytes, bool]:
        if method == "GET" and path == "/redirect":
            self._stages.add("redirect_sent")
            return b"HTTP/1.1 302 Found\r\nLocation: /chunked\r\nContent-Length: 0\r\n\r\n", False
        if method == "GET" and path == "/chunked":
            self._stages.add("chunked_sent")
            return (
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n"
                b"7\r\nchunked\r\n15\r\n-body-from-e1000-peer\r\n0\r\n\r\n",
                False,
            )
        if method == "GET" and path == "/gzip":
            self._stages.add("gzip_sent")
            gz = gzip.compress(GZIP_DATA, mtime=0)
            hdr = f"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {len(gz)}\r\n\r\n".encode()
            return hdr + gz, False
        if method == "POST" and path == "/echo":
            if body != b"post-ping":
                raise ValueError(f"unexpected POST body: {body!r}")
            self._stages.add("post_sent")
            out = b"echo:" + body
            hdr = f"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {len(out)}\r\n\r\n".encode()
            return hdr + out, True
        if method == "GET" and path == "/loop":
            self._stages.add("loop_hit")
            return b"HTTP/1.1 302 Found\r\nLocation: /loop\r\nConnection: close\r\nContent-Length: 0\r\n\r\n", True
        if method == "GET" and path == "/bomb":
            self._stages.add("bomb_sent")
            gz = gzip.compress(GZIP_BOMB, mtime=0)
            hdr = f"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nConnection: close\r\nContent-Length: {len(gz)}\r\n\r\n".encode()
            return hdr + gz, True
        if method == "GET" and path == "/truncated":
            self._stages.add("truncated_sent")
            return b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nhi", True
        raise ValueError(f"unexpected HTTP request: {method} {path}")

    def _finish_exchange(self) -> dict:
        return {
            "protocols": ["ARP", "UDP", "DNS", "TCP", "HTTP/1.1"],
            "stages": sorted(self._stages),
        }

