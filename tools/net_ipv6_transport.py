"""Dual-NIC E1000 Ethernet verifier for N7 IPv6 SLAAC/RA/RDNSS/DAD, TLS 1.3, Happy Eyeballs, and failover."""
from __future__ import annotations

from ipaddress import IPv6Address
from pathlib import Path
import socket
import ssl
import struct
import threading
from time import monotonic

from net_dhcp_transport import ACK as DHCP_ACK, COOKIE, DISCOVER, OFFER, REQUEST, option, options as parse_dhcp_opts
from net_live_transport import ACK, FIN, PSH, SYN, advance
from net_test_transport import checksum

ROOT = Path(__file__).resolve().parents[1]
PKI_DIR = ROOT / "tests/net-https-pki"

ETH_IPV4 = b"\x08\x00"
ETH_ARP = b"\x08\x06"
ETH_IPV6 = b"\x86\xdd"
BROADCAST_MAC = b"\xff" * 6

# NIC0 (metric 10, IPv6-only SLAAC/RA/RDNSS)
MAC_GUEST0 = bytes.fromhex("525400123456")
MAC_HOST0 = bytes.fromhex("02aabbccdd01")
LL_HOST0 = IPv6Address("fe80::aa:bbff:fecc:dd01").packed
LL_GUEST0 = IPv6Address("fe80::5054:ff:fe12:3456").packed
PFX0 = IPv6Address("2001:db8:1::").packed
GUA_GUEST0 = IPv6Address("2001:db8:1::5054:ff:fe12:3456").packed
DNS0_V6 = IPv6Address("2001:db8:1::53").packed
HTTPS0_V6 = IPv6Address("2001:db8:1::443").packed
PEER0_V6 = IPv6Address("2001:db8:1::90").packed
DEAD0_V6 = IPv6Address("2001:db8:1::dead").packed

# NIC1 (metric 20, Dual-Stack DHCPv4 + SLAAC/RA/RDNSS)
MAC_GUEST1 = bytes.fromhex("525400123457")
MAC_HOST1 = bytes.fromhex("02aabbccdd02")
LL_HOST1 = IPv6Address("fe80::aa:bbff:fecc:dd02").packed
PFX1 = IPv6Address("2001:db8:2::").packed
GUA_GUEST1 = IPv6Address("2001:db8:2::5054:ff:fe12:3457").packed
DNS1_V6 = IPv6Address("2001:db8:2::53").packed
FAILOVER1_V6 = IPv6Address("2001:db8:99::81").packed

V4_SERVER = bytes((198, 51, 100, 1))
V4_GUEST1 = bytes((198, 51, 100, 12))
V4_MASK = bytes((255, 255, 255, 0))
V4_DNS1 = bytes((198, 51, 100, 53))
V4_HAPPY1 = bytes((198, 51, 100, 80))
V4_FAILOVER1 = bytes((203, 0, 113, 81))

ALL_NODES = IPv6Address("ff02::1").packed
ALL_NODES_MAC = b"\x33\x33\x00\x00\x00\x01"
ALL_ROUTERS = IPv6Address("ff02::2").packed
UNSPEC_V6 = bytes(16)
MSS_V6 = 1440
MSS_V4 = 1460


def eth_frame(dst: bytes, src: bytes, etype: bytes, payload: bytes) -> bytes:
    return (dst + src + etype + payload).ljust(60, b"\0")


def pseudo_v4(src: bytes, dst: bytes, proto: int, length: int) -> bytes:
    return struct.pack("!4s4sBBH", src, dst, 0, proto, length)


def pseudo_v6(src: bytes, dst: bytes, proto: int, length: int) -> bytes:
    return src + dst + struct.pack("!II", length, proto)


def wrap_v4(proto: int, src: bytes, dst: bytes, payload: bytes) -> bytes:
    hdr = struct.pack("!BBHHHBBH4s4s", 0x45, 0, 20 + len(payload), 0, 0x4000, 64, proto, 0, src, dst)
    return hdr[:10] + struct.pack("!H", checksum(hdr)) + hdr[12:] + payload


def wrap_v6(proto: int, hop: int, src: bytes, dst: bytes, payload: bytes) -> bytes:
    hdr = struct.pack("!IHBB16s16s", 0x60000000, len(payload), proto, hop, src, dst)
    return hdr + payload


def icmpv6_with_csum(src: bytes, dst: bytes, body: bytearray) -> bytes:
    c = checksum(pseudo_v6(src, dst, 58, len(body)) + bytes(body))
    struct.pack_into("!H", body, 2, c)
    return wrap_v6(58, 255, src, dst, bytes(body))


def build_ra(src_ll: bytes, src_mac: bytes, prefix: bytes, rdnss: bytes) -> bytes:
    body = bytearray(16)
    body[0] = 134
    body[4] = 64
    struct.pack_into("!H", body, 6, 1800)
    body += struct.pack("!BB6s", 1, 1, src_mac)
    body += struct.pack("!BBBBII4s16s", 3, 4, 64, 0xC0, 86400, 14400, bytes(4), prefix)
    body += struct.pack("!BBHI16s", 25, 3, 0, 3600, rdnss)
    return eth_frame(ALL_NODES_MAC, src_mac, ETH_IPV6, icmpv6_with_csum(src_ll, ALL_NODES, body))


def build_na(src: bytes, dst: bytes, src_mac: bytes, dst_mac: bytes, target: bytes, solicited: bool) -> bytes:
    body = bytearray(32)
    body[0] = 136
    body[4] = (0x40 if solicited else 0) | 0x20
    body[8:24] = target
    body[24:32] = struct.pack("!BB6s", 2, 1, src_mac)
    return eth_frame(dst_mac, src_mac, ETH_IPV6, icmpv6_with_csum(src, dst, body))


def dns_encode_name(name: bytes) -> bytes:
    out = bytearray()
    for part in name.split(b"."):
        out.append(len(part))
        out.extend(part)
    out.append(0)
    return bytes(out)


def parse_dns_question(msg: bytes) -> tuple[bytes, bytes, int]:
    pos = 12
    labels = []
    while pos < len(msg):
        l = msg[pos]
        pos += 1
        if l == 0:
            break
        labels.append(msg[pos:pos + l].lower())
        pos += l
    qtype = struct.unpack_from("!H", msg, pos)[0]
    return msg[:2], b".".join(labels), qtype


def dns_reply_rr(qid: bytes, qname: bytes, qtype: int, ttl: int, rdata: bytes) -> bytes:
    q = dns_encode_name(qname) + struct.pack("!HH", qtype, 1)
    hdr = qid + struct.pack("!5H", 0x8180, 1, 1, 0, 0)
    ans = struct.pack("!HHHIH", 0xC00C, qtype, 1, ttl, len(rdata)) + rdata
    return hdr + q + ans


def dns_reply_error(qid: bytes, qname: bytes, qtype: int, rcode: int) -> bytes:
    q = dns_encode_name(qname) + struct.pack("!HH", qtype, 1)
    flags = 0x8180 | (rcode & 0x0F)
    return qid + struct.pack("!5H", flags, 1, 0, 0, 0) + q


def host_udp_v6(src_mac: bytes, dst_mac: bytes, src: bytes, dst: bytes, sport: int, dport: int, data: bytes) -> bytes:
    length = 8 + len(data)
    dgram = struct.pack("!HHHH", sport, dport, length, 0) + data
    chk = checksum(pseudo_v6(src, dst, 17, length) + dgram) or 0xFFFF
    return eth_frame(dst_mac, src_mac, ETH_IPV6, wrap_v6(17, 64, src, dst, dgram[:6] + struct.pack("!H", chk) + dgram[8:]))


def host_tcp_v6(src_mac: bytes, dst_mac: bytes, src: bytes, dst: bytes, sport: int, dport: int, seq: int, ack: int, flags: int, data: bytes = b"") -> bytes:
    opts = struct.pack("!BBH", 2, 4, MSS_V6) if (flags & SYN) else b""
    off = (5 + len(opts) // 4) << 4
    hdr = struct.pack("!HHIIBBHHH", sport, dport, seq, ack, off, flags, 16384, 0, 0) + opts
    seg = hdr + data
    chk = checksum(pseudo_v6(src, dst, 6, len(seg)) + seg)
    return eth_frame(dst_mac, src_mac, ETH_IPV6, wrap_v6(6, 64, src, dst, seg[:16] + struct.pack("!H", chk) + seg[18:]))


def host_udp_v4(src_mac: bytes, dst_mac: bytes, src: bytes, dst: bytes, sport: int, dport: int, data: bytes) -> bytes:
    length = 8 + len(data)
    dgram = struct.pack("!HHHH", sport, dport, length, 0) + data
    chk = checksum(pseudo_v4(src, dst, 17, length) + dgram) or 0xFFFF
    return eth_frame(dst_mac, src_mac, ETH_IPV4, wrap_v4(17, src, dst, dgram[:6] + struct.pack("!H", chk) + dgram[8:]))


def host_tcp_v4(src_mac: bytes, dst_mac: bytes, src: bytes, dst: bytes, sport: int, dport: int, seq: int, ack: int, flags: int, data: bytes = b"") -> bytes:
    opts = struct.pack("!BBH", 2, 4, MSS_V4) if (flags & SYN) else b""
    off = (5 + len(opts) // 4) << 4
    hdr = struct.pack("!HHIIBBHHH", sport, dport, seq, ack, off, flags, 16384, 0, 0) + opts
    seg = hdr + data
    chk = checksum(pseudo_v4(src, dst, 6, len(seg)) + seg)
    return eth_frame(dst_mac, src_mac, ETH_IPV4, wrap_v4(6, src, dst, seg[:16] + struct.pack("!H", chk) + seg[18:]))


class SocketSide:
    def __init__(self, name: str):
        self.name = name
        self.endpoint = ""
        self.errors: list[str] = []
        self._listener: socket.socket | None = None
        self._conn: socket.socket | None = None
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        self._lock = threading.Lock()

    def start(self, handler) -> None:
        l = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        l.bind(("127.0.0.1", 0))
        l.listen(1)
        l.settimeout(0.2)
        self._listener = l
        self.endpoint = f"127.0.0.1:{l.getsockname()[1]}"
        self._thread = threading.Thread(target=self._run, args=(handler,), name=f"peer-{self.name}", daemon=True)
        self._thread.start()

    def _run(self, handler) -> None:
        try:
            assert self._listener is not None
            conn = None
            while not self._stop.is_set() and conn is None:
                try:
                    conn, _ = self._listener.accept()
                except socket.timeout:
                    continue
            if conn is None:
                return
            conn.settimeout(0.2)
            self._conn = conn
            handler(self, conn)
        except Exception as exc:
            if not self._stop.is_set():
                self.errors.append(f"{self.name}: {exc}")

    def read_frame(self, conn: socket.socket) -> bytes | None:
        hdr = bytearray()
        while len(hdr) < 4 and not self._stop.is_set():
            try:
                chunk = conn.recv(4 - len(hdr))
            except socket.timeout:
                continue
            if not chunk:
                return None
            hdr.extend(chunk)
        if len(hdr) != 4:
            return None
        size = struct.unpack("!I", bytes(hdr))[0]
        body = bytearray()
        while len(body) < size and not self._stop.is_set():
            try:
                chunk = conn.recv(size - len(body))
            except socket.timeout:
                continue
            if not chunk:
                return None
            body.extend(chunk)
        return bytes(body)

    def send_frame(self, conn: socket.socket, frame: bytes) -> None:
        with self._lock:
            conn.sendall(struct.pack("!I", len(frame)) + frame)

    def stop(self) -> None:
        self._stop.set()
        if self._thread:
            self._thread.join(timeout=5)
        for sock in (self._conn, self._listener):
            if sock:
                try:
                    sock.close()
                except OSError:
                    pass


class NetIpv6Peer:
    def __init__(self, workdir: Path):
        self.workdir = Path(workdir)
        self.side0 = SocketSide("nic0")
        self.side1 = SocketSide("nic1")
        self.endpoint0 = ""
        self.endpoint1 = ""
        self._done = threading.Event()
        self._stages: set[str] = set()
        self._tls_ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        self._tls_ctx.minimum_version = ssl.TLSVersion.TLSv1_3
        self._tls_ctx.maximum_version = ssl.TLSVersion.TLSv1_3
        self._tls_ctx.load_cert_chain(str(PKI_DIR / "valid.pem"), str(PKI_DIR / "valid.key"))

    def start(self) -> None:
        self.side0.start(self._serve0)
        self.side1.start(self._serve1)
        self.endpoint0 = self.side0.endpoint
        self.endpoint1 = self.side1.endpoint

    def wait_complete(self, timeout: float) -> bool:
        expected = {
            "ll_dad",
            "rs_ra0",
            "slaac_dad",
            "icmpv6_echo",
            "tls13_v6",
            "http_v6_literal",
            "tcp6_listener",
            "udp6_echo",
            "happy_eyeballs_syn_v6",
            "happy_eyeballs_v4",
            "posix6_tcp",
            "posix6_udp",
            "nic1_dhcp",
            "nic1_ra",
            "nic1_v6_failover",
            "nic1_v4_failover",
        }
        deadline = monotonic() + timeout
        while monotonic() < deadline:
            if expected.issubset(self._stages):
                return True
            self._done.wait(0.1)
            self._done.clear()
        return expected.issubset(self._stages)

    def stop(self) -> dict:
        self.side0.stop()
        self.side1.stop()
        errors = self.side0.errors + self.side1.errors
        return {
            "ok": not errors,
            "errors": errors,
            "stages": sorted(self._stages),
        }

    def _mark(self, stage: str) -> None:
        self._stages.add(stage)
        self._done.set()

    def _serve0(self, side: SocketSide, conn: socket.socket) -> None:
        tcp_conns: dict[tuple[bytes, int, int], dict] = {}
        active_client: dict[int, dict] = {}
        ping_sent = False

        while not side._stop.is_set():
            frame = side.read_frame(conn)
            if frame is None:
                return
            if len(frame) < 54 or frame[12:14] != ETH_IPV6:
                continue
            src_mac = frame[6:12]
            if src_mac != MAC_GUEST0:
                continue
            ip = frame[14:]
            plen = struct.unpack_from("!H", ip, 4)[0]
            nh, hop = ip[6], ip[7]
            src6, dst6 = ip[8:24], ip[24:40]
            payload = ip[40:40 + plen]

            if not ping_sent and src6 == GUA_GUEST0:
                ping_sent = True
                echo = bytearray(b"\x80\x00\x00\x00\x60\x01\x00\x01ping6")
                side.send_frame(conn, eth_frame(MAC_GUEST0, MAC_HOST0, ETH_IPV6, icmpv6_with_csum(DNS0_V6, GUA_GUEST0, echo).replace(bytes([255]), bytes([64]), 1) if False else wrap_v6(58, 64, DNS0_V6, GUA_GUEST0, self._icmp_csum_bytes(DNS0_V6, GUA_GUEST0, echo))))

            if nh == 58 and hop == 255:
                kind = payload[0]
                if kind == 135:  # NS
                    target = payload[8:24]
                    if src6 == UNSPEC_V6:
                        if target == LL_GUEST0:
                            self._mark("ll_dad")
                        elif target == GUA_GUEST0:
                            self._mark("slaac_dad")
                    elif target in (LL_HOST0, DNS0_V6, HTTPS0_V6, PEER0_V6, DEAD0_V6):
                        side.send_frame(conn, build_na(target, src6, MAC_HOST0, MAC_GUEST0, target, True))
                elif kind == 133:  # RS
                    side.send_frame(conn, build_ra(LL_HOST0, MAC_HOST0, PFX0, DNS0_V6))
                    self._mark("rs_ra0")
                elif kind == 129 and payload[4:8] == b"\x60\x01\x00\x01":
                    self._mark("icmpv6_echo")
                continue

            if nh == 58 and payload[0] == 129 and payload[4:8] == b"\x60\x01\x00\x01":
                self._mark("icmpv6_echo")
                continue

            if nh == 17 and dst6 == DNS0_V6:
                sport, dport = struct.unpack_from("!HH", payload, 0)
                if dport != 53:
                    continue
                dns = payload[8:]
                qid, qname, qtype = parse_dns_question(dns)
                if qname == b"https.du.net.test":
                    ans = dns_reply_rr(qid, qname, 28, 60, HTTPS0_V6) if qtype == 28 else dns_reply_error(qid, qname, qtype, 3)
                elif qname == b"happy.du.net.test":
                    ans = dns_reply_rr(qid, qname, 28, 60, DEAD0_V6) if qtype == 28 else dns_reply_rr(qid, qname, 1, 60, V4_HAPPY1)
                elif qname == b"posix6.du.net.test":
                    ans = dns_reply_rr(qid, qname, 28, 60, PEER0_V6) if qtype == 28 else dns_reply_rr(qid, qname, 1, 60, bytes((198, 51, 100, 90)))
                elif qname == b"failover6.du.net.test":
                    ans = dns_reply_error(qid, qname, qtype, 2)  # SERVFAIL until nic0 goes down
                else:
                    ans = dns_reply_error(qid, qname, qtype, 3)
                side.send_frame(conn, host_udp_v6(MAC_HOST0, MAC_GUEST0, DNS0_V6, src6, 53, sport, ans))
                continue

            if nh == 17 and dst6 == PEER0_V6:
                sport, dport = struct.unpack_from("!HH", payload, 0)
                data = payload[8:]
                if dport == 7001 and data == b"listener6-ready":
                    side.send_frame(conn, host_udp_v6(MAC_HOST0, MAC_GUEST0, PEER0_V6, src6, 7001, sport, b"udp6-pong"))
                    self._mark("udp6_echo")
                    # Connect back to guest [GUA_GUEST0]:8081
                    c_seq = 0x51000001
                    active_client[41111] = {"src": src6, "host_seq": c_seq, "guest_next": 0, "buf": bytearray()}
                    side.send_frame(conn, host_tcp_v6(MAC_HOST0, MAC_GUEST0, PEER0_V6, src6, 41111, 8081, c_seq - 1, 0, SYN))
                elif dport == 7001 and data == b"posix6-udp-ping":
                    side.send_frame(conn, host_udp_v6(MAC_HOST0, MAC_GUEST0, PEER0_V6, src6, 7001, sport, b"posix6-udp-pong"))
                    self._mark("posix6_udp")
                continue

            if nh == 6:
                sport, dport, seq, ack, off, flags = struct.unpack_from("!HHIIBB", payload, 0)
                data = payload[(off >> 4) * 4:]
                if dst6 == DEAD0_V6 and (flags & SYN):
                    self._mark("happy_eyeballs_syn_v6")
                    continue
                if sport == 8081 and dport in active_client and src6 == active_client[dport]["src"]:
                    st = active_client[dport]
                    if (flags & (SYN | ACK)) == (SYN | ACK):
                        st["guest_next"] = advance(seq, 1)
                        side.send_frame(conn, host_tcp_v6(MAC_HOST0, MAC_GUEST0, PEER0_V6, src6, dport, sport, st["host_seq"], st["guest_next"], ACK))
                        msg = b"hello-listener6"
                        s = st["host_seq"]
                        st["host_seq"] = advance(s, len(msg))
                        side.send_frame(conn, host_tcp_v6(MAC_HOST0, MAC_GUEST0, PEER0_V6, src6, dport, sport, s, st["guest_next"], PSH | ACK, msg))
                    elif data or (flags & FIN):
                        if data:
                            st["guest_next"] = advance(seq, len(data))
                            st["buf"].extend(data)
                        if flags & FIN:
                            st["guest_next"] = advance(st["guest_next"], 1)
                        side.send_frame(conn, host_tcp_v6(MAC_HOST0, MAC_GUEST0, PEER0_V6, src6, dport, sport, st["host_seq"], st["guest_next"], ACK | (FIN if (flags & FIN) else 0)))
                        if bytes(st["buf"]) == b"listener6-ack" and (flags & FIN):
                            st["host_seq"] = advance(st["host_seq"], 1)
                            self._mark("tcp6_listener")
                    continue

                key = (dst6, dport, sport)
                if flags == SYN:
                    gnext = advance(seq, 1)
                    hseq = 0x41000001
                    c = {"sport": sport, "dport": dport, "dst": dst6, "guest": src6, "guest_next": gnext, "host_seq": hseq, "buf": bytearray(), "done": False}
                    if dst6 == HTTPS0_V6 and dport == 443:
                        ib, ob = ssl.MemoryBIO(), ssl.MemoryBIO()
                        c["bio"] = (ib, ob, self._tls_ctx.wrap_bio(ib, ob, server_side=True))
                    tcp_conns[key] = c
                    side.send_frame(conn, host_tcp_v6(MAC_HOST0, MAC_GUEST0, dst6, src6, dport, sport, hseq - 1, gnext, SYN | ACK))
                elif key in tcp_conns:
                    c = tcp_conns[key]
                    if data:
                        c["guest_next"] = advance(seq, len(data))
                        self._on_tcp0_data(side, conn, c, data)
                    if flags & FIN:
                        c["guest_next"] = advance(c["guest_next"], 1)
                        side.send_frame(conn, host_tcp_v6(MAC_HOST0, MAC_GUEST0, c["dst"], c["guest"], c["dport"], c["sport"], c["host_seq"], c["guest_next"], ACK))
                    elif data:
                        pass

    def _icmp_csum_bytes(self, src: bytes, dst: bytes, body: bytearray) -> bytes:
        c = checksum(pseudo_v6(src, dst, 58, len(body)) + bytes(body))
        struct.pack_into("!H", body, 2, c)
        return bytes(body)

    def _send_v6_chunks(self, side: SocketSide, conn: socket.socket, c: dict, out: bytes, *, fin: bool = False) -> None:
        if not out:
            flags = ACK | (FIN if fin else 0)
            side.send_frame(conn, host_tcp_v6(MAC_HOST0, MAC_GUEST0, c["dst"], c["guest"], c["dport"], c["sport"], c["host_seq"], c["guest_next"], flags))
            if fin:
                c["host_seq"] = advance(c["host_seq"], 1)
            return
        off = 0
        while off < len(out):
            chunk = out[off:off + MSS_V6]
            last = (off + len(chunk) == len(out))
            flags = PSH | ACK | (FIN if (fin and last) else 0)
            seq = c["host_seq"]
            c["host_seq"] = advance(seq, len(chunk) + (1 if (fin and last) else 0))
            side.send_frame(conn, host_tcp_v6(MAC_HOST0, MAC_GUEST0, c["dst"], c["guest"], c["dport"], c["sport"], seq, c["guest_next"], flags, chunk))
            off += len(chunk)

    def _on_tcp0_data(self, side: SocketSide, conn: socket.socket, c: dict, data: bytes) -> None:
        if "bio" in c:
            ib, ob, obj = c["bio"]
            ib.write(data)
            try:
                if not c.get("hs_done"):
                    try:
                        obj.do_handshake()
                        c["hs_done"] = True
                    except (ssl.SSLWantReadError, ssl.SSLWantWriteError):
                        pass
                if c.get("hs_done") and not c["done"]:
                    try:
                        req = obj.read(4096)
                    except (ssl.SSLWantReadError, ssl.SSLWantWriteError):
                        req = b""
                    if req:
                        c["buf"].extend(req)
                        if b"\r\n\r\n" in c["buf"]:
                            resp = b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\nConnection: close\r\n\r\nipv6-tls13-ok"
                            obj.write(resp)
                            c["done"] = True
                            self._mark("tls13_v6")
            except ssl.SSLError:
                c["done"] = True
            pending = ob.read()
            self._send_v6_chunks(side, conn, c, pending, fin=c["done"])
        elif c["dst"] == HTTPS0_V6 and c["dport"] == 8082:
            c["buf"].extend(data)
            if b"\r\n\r\n" in c["buf"] and not c["done"]:
                c["done"] = True
                resp = b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\nipv6-http-literal-ok"
                self._send_v6_chunks(side, conn, c, resp, fin=True)
                self._mark("http_v6_literal")
        elif c["dst"] == PEER0_V6 and c["dport"] == 7000:
            c["buf"].extend(data)
            if bytes(c["buf"]) == b"posix6-tcp-ping" and not c["done"]:
                c["done"] = True
                self._send_v6_chunks(side, conn, c, b"posix6-tcp-pong", fin=True)
                self._mark("posix6_tcp")

    def _serve1(self, side: SocketSide, conn: socket.socket) -> None:
        tcp4: dict[tuple[bytes, int, int], dict] = {}
        tcp6: dict[tuple[bytes, int, int], dict] = {}

        while not side._stop.is_set():
            frame = side.read_frame(conn)
            if frame is None:
                return
            if len(frame) < 42 or frame[6:12] != MAC_GUEST1:
                continue
            eth = frame[12:14]

            if eth == ETH_ARP:
                arp_body = frame[14:42]
                op = struct.unpack_from("!H", arp_body, 6)[0]
                spa, tpa = arp_body[14:18], arp_body[24:28]
                if op == 1 and tpa in (V4_SERVER, V4_DNS1, V4_HAPPY1, V4_FAILOVER1) and spa == V4_GUEST1:
                    rep = struct.pack("!HHBBH6s4s6s4s", 1, 0x0800, 6, 4, 2, MAC_HOST1, tpa, MAC_GUEST1, spa)
                    side.send_frame(conn, eth_frame(MAC_GUEST1, MAC_HOST1, ETH_ARP, rep))
                continue

            if eth == ETH_IPV4:
                total = struct.unpack_from("!H", frame, 16)[0]
                proto = frame[23]
                src4, dst4 = frame[26:30], frame[30:34]
                payload = frame[34:14 + total]
                if proto == 17:
                    sport, dport = struct.unpack_from("!HH", payload, 0)
                    if sport == 68 and dport == 67:
                        bootp = payload[8:]
                        xid = struct.unpack_from("!I", bootp, 4)[0]
                        opts = parse_dhcp_opts(bootp[240:])
                        msg_type = opts.get(53, b"")
                        if msg_type == bytes([DISCOVER]):
                            side.send_frame(conn, self._dhcp_reply(OFFER, xid))
                        elif msg_type == bytes([REQUEST]):
                            side.send_frame(conn, self._dhcp_reply(DHCP_ACK, xid))
                            self._mark("nic1_dhcp")
                    elif dst4 == V4_DNS1 and dport == 53:
                        dns = payload[8:]
                        qid, qname, qtype = parse_dns_question(dns)
                        if qname == b"failover6.du.net.test" and qtype == 1:
                            ans = dns_reply_rr(qid, qname, 1, 60, V4_FAILOVER1)
                        elif qname == b"failover6.du.net.test" and qtype == 28:
                            ans = dns_reply_rr(qid, qname, 28, 60, FAILOVER1_V6)
                        else:
                            ans = dns_reply_error(qid, qname, qtype, 3)
                        side.send_frame(conn, host_udp_v4(MAC_HOST1, MAC_GUEST1, V4_DNS1, src4, 53, sport, ans))
                elif proto == 6:
                    sport, dport, seq, ack, off, flags = struct.unpack_from("!HHIIBB", payload, 0)
                    data = payload[(off >> 4) * 4:]
                    key = (dst4, dport, sport)
                    if flags == SYN and dst4 in (V4_HAPPY1, V4_FAILOVER1) and dport == 80:
                        gnext = advance(seq, 1)
                        hseq = 0x72000001
                        tcp4[key] = {"dst": dst4, "sport": sport, "dport": dport, "guest": src4, "guest_next": gnext, "host_seq": hseq, "buf": bytearray(), "done": False}
                        side.send_frame(conn, host_tcp_v4(MAC_HOST1, MAC_GUEST1, dst4, src4, dport, sport, hseq - 1, gnext, SYN | ACK))
                    elif key in tcp4:
                        c = tcp4[key]
                        if data:
                            c["guest_next"] = advance(seq, len(data))
                            c["buf"].extend(data)
                            if b"\r\n\r\n" in c["buf"] and not c["done"]:
                                c["done"] = True
                                if c["dst"] == V4_HAPPY1:
                                    body = b"happy-eyeballs-v4-fallback"
                                    self._mark("happy_eyeballs_v4")
                                else:
                                    body = b"nic1-v4-failover-ok"
                                    self._mark("nic1_v4_failover")
                                resp = f"HTTP/1.1 200 OK\r\nContent-Length: {len(body)}\r\nConnection: close\r\n\r\n".encode() + body
                                hseq = c["host_seq"]
                                c["host_seq"] = advance(hseq, len(resp) + 1)
                                side.send_frame(conn, host_tcp_v4(MAC_HOST1, MAC_GUEST1, c["dst"], c["guest"], c["dport"], c["sport"], hseq, c["guest_next"], PSH | ACK | FIN, resp))
                        elif flags & FIN:
                            c["guest_next"] = advance(c["guest_next"], 1)
                            side.send_frame(conn, host_tcp_v4(MAC_HOST1, MAC_GUEST1, c["dst"], c["guest"], c["dport"], c["sport"], c["host_seq"], c["guest_next"], ACK))
                continue

            if eth == ETH_IPV6:
                ip = frame[14:]
                plen = struct.unpack_from("!H", ip, 4)[0]
                nh, hop = ip[6], ip[7]
                src6, dst6 = ip[8:24], ip[24:40]
                payload = ip[40:40 + plen]
                if nh == 58 and hop == 255:
                    kind = payload[0]
                    if kind == 135:
                        target = payload[8:24]
                        if src6 != UNSPEC_V6 and target in (LL_HOST1, DNS1_V6, FAILOVER1_V6):
                            side.send_frame(conn, build_na(target, src6, MAC_HOST1, MAC_GUEST1, target, True))
                    elif kind == 133:
                        side.send_frame(conn, build_ra(LL_HOST1, MAC_HOST1, PFX1, DNS1_V6))
                        self._mark("nic1_ra")
                elif nh == 17 and dst6 == DNS1_V6:
                    sport, dport = struct.unpack_from("!HH", payload, 0)
                    if dport == 53:
                        dns = payload[8:]
                        qid, qname, qtype = parse_dns_question(dns)
                        if qname == b"failover6.du.net.test":
                            ans = dns_reply_rr(qid, qname, 28, 60, FAILOVER1_V6) if qtype == 28 else dns_reply_rr(qid, qname, 1, 60, V4_FAILOVER1)
                        else:
                            ans = dns_reply_error(qid, qname, qtype, 3)
                        side.send_frame(conn, host_udp_v6(MAC_HOST1, MAC_GUEST1, DNS1_V6, src6, 53, sport, ans))
                elif nh == 6 and dst6 == FAILOVER1_V6:
                    sport, dport, seq, ack, off, flags = struct.unpack_from("!HHIIBB", payload, 0)
                    data = payload[(off >> 4) * 4:]
                    key = (dst6, dport, sport)
                    if flags == SYN and dport == 80:
                        gnext = advance(seq, 1)
                        hseq = 0x82000001
                        tcp6[key] = {"dst": dst6, "sport": sport, "dport": dport, "guest": src6, "guest_next": gnext, "host_seq": hseq, "buf": bytearray(), "done": False}
                        side.send_frame(conn, host_tcp_v6(MAC_HOST1, MAC_GUEST1, dst6, src6, dport, sport, hseq - 1, gnext, SYN | ACK))
                    elif key in tcp6:
                        c = tcp6[key]
                        if data:
                            c["guest_next"] = advance(seq, len(data))
                            c["buf"].extend(data)
                            if b"\r\n\r\n" in c["buf"] and not c["done"]:
                                c["done"] = True
                                body = b"nic1-v6-failover-ok"
                                resp = f"HTTP/1.1 200 OK\r\nContent-Length: {len(body)}\r\nConnection: close\r\n\r\n".encode() + body
                                hseq = c["host_seq"]
                                c["host_seq"] = advance(hseq, len(resp) + 1)
                                side.send_frame(conn, host_tcp_v6(MAC_HOST1, MAC_GUEST1, c["dst"], c["guest"], c["dport"], c["sport"], hseq, c["guest_next"], PSH | ACK | FIN, resp))
                                self._mark("nic1_v6_failover")
                        elif flags & FIN:
                            c["guest_next"] = advance(c["guest_next"], 1)
                            side.send_frame(conn, host_tcp_v6(MAC_HOST1, MAC_GUEST1, c["dst"], c["guest"], c["dport"], c["sport"], c["host_seq"], c["guest_next"], ACK))

    def _dhcp_reply(self, kind: int, xid: int) -> bytes:
        body = bytearray(240)
        body[:4] = b"\x02\x01\x06\x00"
        struct.pack_into("!I", body, 4, xid)
        body[16:20] = V4_GUEST1
        body[28:34] = MAC_GUEST1
        body[236:240] = COOKIE
        body += option(53, bytes([kind])) + option(54, V4_SERVER)
        body += option(1, V4_MASK) + option(3, V4_SERVER) + option(6, V4_DNS1)
        body += option(51, struct.pack("!I", 60))
        body += b"\xff"
        body = body.ljust(300, b"\0")
        length = 8 + len(body)
        dgram = struct.pack("!HHHH", 67, 68, length, 0) + bytes(body)
        chk = checksum(pseudo_v4(V4_SERVER, b"\xff" * 4, 17, length) + dgram) or 0xFFFF
        payload = dgram[:6] + struct.pack("!H", chk) + dgram[8:]
        return eth_frame(BROADCAST_MAC, MAC_HOST1, ETH_IPV4, wrap_v4(17, V4_SERVER, b"\xff" * 4, payload))

