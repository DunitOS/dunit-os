"""Isolated E1000 Real-OpenSSL TLS 1.3 + HTTP CONNECT proxy + hostile browser test peer."""
from __future__ import annotations

import gzip
from pathlib import Path
import socket
import ssl
import struct
from threading import Event

from net_dhcp_transport import ARP, BROADCAST_MAC, IPV4, ipv4
from net_dns_transport import DNS_PORT, host_udp, parse_tcp, parse_udp
from net_live_transport import ACK, FIN, PSH, SYN, advance, dns_answer, pseudo
from net_static_transport import arp, ethernet
from net_test_transport import EthernetPeer, GUEST_MAC, HOST_MAC, checksum

ROOT = Path(__file__).resolve().parents[1]
PKI_DIR = ROOT / "tests/net-https-pki"

GUEST = bytes((192, 0, 2, 12))
DNS_HOST = bytes((192, 0, 2, 1))
HTTPS_HOST = bytes((192, 0, 2, 2))
MSS = 1460
RST = 0x04

Q_HTTPS = b"\x05https\x02du\x03net\x04test\x00\x00\x01\x00\x01"
Q_EXPIRED = b"\x07expired\x02du\x03net\x04test\x00\x00\x01\x00\x01"
VIEWER_BODY = b"Dunit OS Isolated Browser Viewer: TLS 1.3 verified response body!"
GZIP_BOMB = gzip.compress(b"A" * (96 * 1024), mtime=0)

PORT_CERTS = {
    443: ("valid.pem", "valid.key"),
    3128: ("valid.pem", "valid.key"),
    8444: ("wrong.pem", "wrong.key"),
    8445: ("expired.pem", "expired.key"),
    8446: ("untrusted.pem", "untrusted.key"),
}


def host_tcp_tls(src_port: int, dst_port: int, seq: int, ack: int, flags: int, data: bytes = b"") -> bytes:
    opts = struct.pack("!BBH", 2, 4, MSS) if (flags & SYN) else b""
    off = (5 + len(opts) // 4) << 4
    hdr = struct.pack("!HHIIBBHHH", src_port, dst_port, seq, ack, off, flags, 16384, 0, 0) + opts
    seg = hdr + data
    chk = checksum(pseudo(HTTPS_HOST, GUEST, 6, len(seg)) + seg)
    ip = ipv4(6, HTTPS_HOST, GUEST, seg[:16] + struct.pack("!H", chk) + seg[18:])
    return ethernet(GUEST_MAC, HOST_MAC, IPV4, ip)


class NetBrowserPeer(EthernetPeer):
    """Serves OpenSSL TLS 1.3 + CONNECT proxy + hostile HTTP/TLS edge cases over E1000."""

    def __init__(self, workdir):
        super().__init__(workdir)
        self._done = Event()
        self._stages: set[str] = set()
        self._contexts: dict[int, ssl.SSLContext] = {}
        for port, (cert_name, key_name) in PORT_CERTS.items():
            ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            ctx.minimum_version = ssl.TLSVersion.TLSv1_3
            ctx.maximum_version = ssl.TLSVersion.TLSv1_3
            ctx.load_cert_chain(str(PKI_DIR / cert_name), str(PKI_DIR / key_name))
            self._contexts[port] = ctx

    def wait_complete(self, timeout: float) -> bool:
        expected = {
            "viewer_served",
            "cookie_echoed",
            "download_served",
            "proxy_connect_ok",
            "loop_served",
            "downgrade_served",
            "truncated_sent",
            "bomb_sent",
            "wrong_san_attempted",
            "expired_attempted",
            "untrusted_attempted",
            "done_ack",
        }
        return self._done.wait(timeout) and expected.issubset(self._stages)

    def _exchange(self, connection: socket.socket) -> None:
        try:
            self._serve(connection)
        finally:
            self._done.set()
        self._stop.wait()

    def _send_tcp_chunks(self, connection: socket.socket, conn: dict, payload: bytes, *, fin: bool = False) -> None:
        if not payload:
            flags = ACK | (FIN if fin else 0)
            if fin:
                conn["fin_sent"] = True
            self._send(connection, host_tcp_tls(conn["dport"], conn["sport"], conn["host_seq"], conn["recv_next"], flags))
            if fin:
                conn["host_seq"] = advance(conn["host_seq"], 1)
            return
        offset = 0
        while offset < len(payload):
            chunk = payload[offset:offset + MSS]
            is_last = (offset + len(chunk) == len(payload))
            flags = PSH | ACK | (FIN if (fin and is_last) else 0)
            if fin and is_last:
                conn["fin_sent"] = True
            self._send(connection, host_tcp_tls(conn["dport"], conn["sport"], conn["host_seq"], conn["recv_next"], flags, chunk))
            conn["host_seq"] = advance(conn["host_seq"], len(chunk) + (1 if (fin and is_last) else 0))
            offset += len(chunk)

    def _serve(self, connection: socket.socket) -> None:
        next_host_isn = 0x70000001
        conns: dict[tuple[int, int], dict] = {}

        while True:
            frame = self._receive(connection)
            eth = frame[12:14]
            if eth == ARP:
                if frame == ethernet(BROADCAST_MAC, GUEST_MAC, ARP, arp(1, GUEST_MAC, bytes(4), bytes(6), GUEST)):
                    continue
                if frame == ethernet(BROADCAST_MAC, GUEST_MAC, ARP, arp(1, GUEST_MAC, GUEST, bytes(6), GUEST)):
                    continue
                for target in (DNS_HOST, HTTPS_HOST):
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
                if Q_EXPIRED in dns_bytes:
                    reply = dns_answer(dns_bytes, Q_EXPIRED, 2, False)
                else:
                    reply = dns_answer(dns_bytes, Q_HTTPS, 2, False)
                self._send(connection, host_udp(DNS_HOST, DNS_PORT, sport, reply))
                continue

            if proto != 6:
                continue

            dst, sport, dport, seq, ack, flags, data = parse_tcp(frame)
            if dst != HTTPS_HOST or dport not in self._contexts:
                raise ValueError(f"unexpected TCP {dst!r}:{dport}")
            key = (sport, dport)
            if flags & RST:
                conns.pop(key, None)
                continue

            if flags & SYN:
                isn = next_host_isn
                next_host_isn = advance(next_host_isn, 0x10000)
                in_bio = ssl.MemoryBIO()
                out_bio = ssl.MemoryBIO()
                ssl_obj = self._contexts[dport].wrap_bio(in_bio, out_bio, server_side=True)
                conn = {
                    "sport": sport,
                    "dport": dport,
                    "recv_next": advance(seq, 1),
                    "host_seq": isn,
                    "in_bio": in_bio,
                    "out_bio": out_bio,
                    "ssl": ssl_obj,
                    "hs_done": False,
                    "proxy_established": dport != 3128,
                    "raw_buf": bytearray(),
                    "http_buf": bytearray(),
                    "fin_sent": False,
                }
                conns[key] = conn
                if dport == 8444:
                    self._stages.add("wrong_san_attempted")
                elif dport == 8445:
                    self._stages.add("expired_attempted")
                elif dport == 8446:
                    self._stages.add("untrusted_attempted")
                self._send(connection, host_tcp_tls(dport, sport, isn - 1, conn["recv_next"], SYN | ACK))
                continue

            conn = conns.get(key)
            if conn is None:
                continue

            if data and seq == conn["recv_next"]:
                conn["recv_next"] = advance(conn["recv_next"], len(data))
                force_fin = False

                if not conn["proxy_established"]:
                    conn["raw_buf"].extend(data)
                    end = conn["raw_buf"].find(b"\r\n\r\n")
                    if end >= 0:
                        head = conn["raw_buf"][:end].decode("utf-8", errors="replace")
                        rest = bytes(conn["raw_buf"][end + 4:])
                        conn["raw_buf"].clear()
                        if not head.startswith("CONNECT "):
                            raise ValueError(f"expected CONNECT got {head!r}")
                        conn["proxy_established"] = True
                        self._stages.add("proxy_connect_ok")
                        self._send_tcp_chunks(
                            connection,
                            conn,
                            b"HTTP/1.1 200 Connection Established\r\n\r\n",
                        )
                        if rest:
                            conn["in_bio"].write(rest)
                        else:
                            continue
                    else:
                        self._send_tcp_chunks(connection, conn, b"")
                        continue
                else:
                    conn["in_bio"].write(data)

                try:
                    if not conn["hs_done"]:
                        try:
                            conn["ssl"].do_handshake()
                            conn["hs_done"] = True
                        except ssl.SSLWantReadError:
                            pass
                    if conn["hs_done"]:
                        while True:
                            try:
                                pt = conn["ssl"].read(4096)
                                if not pt:
                                    break
                                conn["http_buf"].extend(pt)
                            except ssl.SSLWantReadError:
                                break
                        while True:
                            req = self._pop_http_request(conn["http_buf"])
                            if req is None:
                                break
                            method, path, headers = req
                            reply_bytes, close_after = self._handle_http(method, path, headers)
                            conn["ssl"].write(reply_bytes)
                            if close_after:
                                force_fin = True
                                break
                except ssl.SSLError:
                    pass

                out = conn["out_bio"].read()
                if out:
                    self._send_tcp_chunks(connection, conn, out, fin=force_fin)
                else:
                    self._send_tcp_chunks(connection, conn, b"", fin=force_fin)
                if "done_ack" in self._stages:
                    return
                continue

            if flags & FIN:
                conn["recv_next"] = advance(seq, len(data) + 1)
                if conn["fin_sent"]:
                    self._send_tcp_chunks(connection, conn, b"")
                else:
                    self._send_tcp_chunks(connection, conn, b"", fin=True)

    def _pop_http_request(self, buf: bytearray) -> tuple[str, str, dict[str, str]] | None:
        end = buf.find(b"\r\n\r\n")
        if end < 0:
            return None
        lines = buf[:end].decode("utf-8").split("\r\n")
        method, path, _ = lines[0].split(" ", 2)
        headers: dict[str, str] = {}
        for line in lines[1:]:
            if ":" in line:
                k, v = line.split(":", 1)
                headers[k.strip().lower()] = v.strip()
        del buf[:end + 4]
        return method, path, headers

    def _handle_http(self, method: str, path: str, headers: dict[str, str]) -> tuple[bytes, bool]:
        if method == "GET" and path == "/viewer":
            self._stages.add("viewer_served")
            hdr = (
                "HTTP/1.1 200 OK\r\n"
                "Content-Type: text/plain\r\n"
                "Cache-Control: max-age=120\r\n"
                "Set-Cookie: sid=alpha42; Secure; HttpOnly; Max-Age=300\r\n"
                f"Content-Length: {len(VIEWER_BODY)}\r\n\r\n"
            ).encode()
            return hdr + VIEWER_BODY, False

        if method == "GET" and path == "/echo-cookie":
            self._stages.add("cookie_echoed")
            val = headers.get("cookie", "none")
            body = f"cookie={val}".encode()
            hdr = f"HTTP/1.1 200 OK\r\nContent-Length: {len(body)}\r\n\r\n".encode()
            return hdr + body, False

        if method == "GET" and path.startswith("/download/"):
            self._stages.add("download_served")
            body = b"safe-download-payload"
            hdr = (
                "HTTP/1.1 200 OK\r\n"
                "Content-Type: application/octet-stream\r\n"
                'Content-Disposition: attachment; filename="../../secret/report.bin"\r\n'
                f"Content-Length: {len(body)}\r\n\r\n"
            ).encode()
            return hdr + body, False

        if method == "GET" and path == "/via-proxy":
            body = b"https-via-connect-proxy-ok"
            hdr = f"HTTP/1.1 200 OK\r\nContent-Length: {len(body)}\r\n\r\n".encode()
            return hdr + body, False

        if method == "GET" and path == "/loop":
            self._stages.add("loop_served")
            return b"HTTP/1.1 302 Found\r\nLocation: /loop\r\nContent-Length: 0\r\n\r\n", False

        if method == "GET" and path == "/downgrade":
            self._stages.add("downgrade_served")
            return b"HTTP/1.1 302 Found\r\nLocation: http://https.du.net.test/\r\nContent-Length: 0\r\n\r\n", False

        if method == "GET" and path == "/truncated":
            self._stages.add("truncated_sent")
            return b"HTTP/1.1 200 OK\r\nContent-Length: 500\r\n\r\ntruncated12", True

        if method == "GET" and path == "/bomb":
            self._stages.add("bomb_sent")
            hdr = (
                "HTTP/1.1 200 OK\r\n"
                "Content-Encoding: gzip\r\n"
                f"Content-Length: {len(GZIP_BOMB)}\r\n\r\n"
            ).encode()
            return hdr + GZIP_BOMB, False

        if method == "GET" and path == "/done":
            self._stages.add("done_ack")
            body = b"done"
            hdr = f"HTTP/1.1 200 OK\r\nContent-Length: {len(body)}\r\n\r\n".encode()
            return hdr + body, False

        raise ValueError(f"unexpected browser HTTPS request: {method} {path}")

    def _finish_exchange(self) -> dict:
        return {
            "protocols": ["ARP", "UDP", "DNS", "TCP", "HTTP CONNECT Proxy", "TLS 1.3 (OpenSSL)", "HTTP/1.1"],
            "stages": sorted(self._stages),
        }

