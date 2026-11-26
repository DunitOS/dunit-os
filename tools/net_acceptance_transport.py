"""M8 acceptance wire peer: ICMPv4/v6 ping, impairment (loss/reorder/dup/corrupt),
malformed garbage injection, TLS 1.3 HTTPS, 24x soak, and post-restart recovery."""
from __future__ import annotations

from ipaddress import IPv6Address
from pathlib import Path
import socket
import ssl
import struct
import threading
import time

from net_live_transport import ACK, ARP, FIN, IPV4, PSH, SYN, advance, pseudo
from net_test_transport import EthernetPeer, checksum

ROOT = Path(__file__).resolve().parents[1]
PKI_DIR = ROOT / "tests/net-https-pki"

MAC_GUEST = bytes.fromhex("525400123456")
MAC_HOST = bytes.fromhex("02aabbccdd01")

IP4_GUEST = bytes((192, 0, 2, 12))
IP4_HOST = bytes((192, 0, 2, 1))
IP4_IMPAIR = bytes((192, 0, 2, 2))
IP4_DNS = bytes((192, 0, 2, 53))

ETH_IPV6 = b"\x86\xdd"
IP6_LL_HOST = IPv6Address("fe80::aa:bbff:fecc:dd01").packed
IP6_PFX = IPv6Address("2001:db8:1::").packed
IP6_DNS = IPv6Address("2001:db8:1::53").packed
IP6_PEER = IPv6Address("2001:db8:1::90").packed
IP6_HTTPS = IPv6Address("2001:db8:1::443").packed
ALL_NODES = IPv6Address("ff02::1").packed
ALL_NODES_MAC = b"\x33\x33\x00\x00\x00\x01"
UNSPEC6 = bytes(16)


def wrap_v4(proto: int, src: bytes, dst: bytes, payload: bytes) -> bytes:
    hdr = struct.pack("!BBHHHBBH4s4s", 0x45, 0, 20 + len(payload), 0, 0x4000, 64, proto, 0, src, dst)
    return hdr[:10] + struct.pack("!H", checksum(hdr)) + hdr[12:] + payload


def wrap_v6(proto: int, hop: int, src: bytes, dst: bytes, payload: bytes) -> bytes:
    return struct.pack("!IHBB16s16s", 0x60000000, len(payload), proto, hop, src, dst) + payload


def pseudo_v6(src: bytes, dst: bytes, proto: int, length: int) -> bytes:
    return src + dst + struct.pack("!II", length, proto)


def host_tcp4(src: bytes, sport: int, dport: int, seq: int, ack: int, flags: int, data: bytes = b"") -> bytes:
    hdr = struct.pack("!HHIIBBHHH", sport, dport, seq, ack, 0x50, flags, 32768, 0, 0) + data
    chk = checksum(pseudo(IP4_GUEST, src, 6, len(hdr)) + hdr)
    return ethernet(MAC_GUEST, IPV4, wrap_v4(6, src, IP4_GUEST, hdr[:16] + struct.pack("!H", chk) + hdr[18:]), source=MAC_HOST)


def ethernet(dest: bytes, ethertype: bytes, payload: bytes, *, source: bytes = MAC_HOST) -> bytes:
    return (dest + source + ethertype + payload).ljust(60, b"\0")


def tcp6_frame(src: bytes, dst: bytes, sport: int, dport: int, seq: int, ack: int, flags: int, data: bytes = b"") -> bytes:
    hdr = struct.pack("!HHIIBBHHH", sport, dport, seq, ack, 0x50, flags, 32768, 0, 0) + data
    chk = checksum(pseudo_v6(src, dst, 6, len(hdr)) + hdr)
    seg = hdr[:16] + struct.pack("!H", chk) + hdr[18:]
    return (MAC_GUEST + MAC_HOST + ETH_IPV6 + wrap_v6(6, 64, src, dst, seg)).ljust(60, b"\0")


def host_udp4(src: bytes, sport: int, dport: int, data: bytes) -> bytes:
    length = 8 + len(data)
    dgram = struct.pack("!HHHH", sport, dport, length, 0) + data
    chk = checksum(pseudo(src, IP4_GUEST, 17, length) + dgram) or 0xFFFF
    return ethernet(MAC_GUEST, IPV4, wrap_v4(17, src, IP4_GUEST, dgram[:6] + struct.pack("!H", chk) + dgram[8:]), source=MAC_HOST)


def parse_dns(msg: bytes) -> tuple[bytes, bytes, int]:
    pos = 12
    parts = []
    while pos < len(msg):
        n = msg[pos]
        pos += 1
        if n == 0:
            break
        parts.append(msg[pos:pos + n].lower())
        pos += n
    qtype = struct.unpack_from("!H", msg, pos)[0]
    return msg[:2], b".".join(parts), qtype


def dns_encode_name(name: bytes) -> bytes:
    return b"".join(bytes([len(p)]) + p for p in name.split(b".")) + b"\x00"


def dns_rr(qid: bytes, name: bytes, qtype: int, rdata: bytes) -> bytes:
    q = dns_encode_name(name) + struct.pack("!HH", qtype, 1)
    hdr = qid + struct.pack("!5H", 0x8180, 1, 1, 0, 0)
    ans = struct.pack("!HHHIH", 0xC00C, qtype, 1, 60, len(rdata)) + rdata
    return hdr + q + ans


def dns_err(qid: bytes, name: bytes, qtype: int, rcode: int) -> bytes:
    q = dns_encode_name(name) + struct.pack("!HH", qtype, 1)
    return qid + struct.pack("!5H", 0x8180 | rcode, 1, 0, 0, 0) + q


def build_ra() -> bytes:
    body = bytearray(16)
    body[0] = 134
    body[4] = 64
    struct.pack_into("!H", body, 6, 1800)
    body += struct.pack("!BB6s", 1, 1, MAC_HOST)
    body += struct.pack("!BBBBII4s16s", 3, 4, 64, 0xC0, 86400, 14400, bytes(4), IP6_PFX)
    body += struct.pack("!BBHI16s", 25, 3, 0, 3600, IP6_DNS)
    csum = checksum(pseudo_v6(IP6_LL_HOST, ALL_NODES, 58, len(body)) + bytes(body))
    struct.pack_into("!H", body, 2, csum)
    return (ALL_NODES_MAC + MAC_HOST + ETH_IPV6 + wrap_v6(58, 255, IP6_LL_HOST, ALL_NODES, bytes(body))).ljust(60, b"\0")


def build_na(src: bytes, dst: bytes, target: bytes) -> bytes:
    body = bytearray(32)
    body[0] = 136
    body[4] = 0x60
    body[8:24] = target
    body[24:32] = struct.pack("!BB6s", 2, 1, MAC_HOST)
    csum = checksum(pseudo_v6(src, dst, 58, len(body)) + bytes(body))
    struct.pack_into("!H", body, 2, csum)
    return (MAC_GUEST + MAC_HOST + ETH_IPV6 + wrap_v6(58, 255, src, dst, bytes(body))).ljust(60, b"\0")


EXPECTED_STAGES = {
    "icmp4",
    "icmp6",
    "syn_dropped",
    "reordered_http",
    "tls13_impairment",
    "soak_24",
    "post_restart_http",
}


class NetAcceptancePeer(EthernetPeer):
    def __init__(self, workdir: Path):
        super().__init__(workdir)
        self._done = threading.Event()
        self._stages: set[str] = set()
        self._tls_ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        self._tls_ctx.minimum_version = ssl.TLSVersion.TLSv1_3
        self._tls_ctx.maximum_version = ssl.TLSVersion.TLSv1_3
        self._tls_ctx.load_cert_chain(str(PKI_DIR / "valid.pem"), str(PKI_DIR / "valid.key"))

    def wait_complete(self, timeout: float) -> bool:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if EXPECTED_STAGES.issubset(self._stages):
                return True
            self._done.wait(0.1)
            self._done.clear()
        return EXPECTED_STAGES.issubset(self._stages)

    def _finish_exchange(self) -> dict:
        missing = sorted(EXPECTED_STAGES - self._stages)
        if missing:
            self.errors.append(f"incomplete M8 acceptance stages: {missing}")
        return {
            "ethertype": "ipv4+ipv6",
            "stages": sorted(self._stages),
        }

    def _mark(self, s: str) -> None:
        self._stages.add(s)
        self._done.set()

    def _inject_garbage(self, conn: socket.socket) -> None:
        # Unknown EtherType, short IPv4, bad IPv4 csum, bad TCP csum.
        self._send(conn, (MAC_GUEST + MAC_HOST + b"\x88\x99" + b"\xff" * 46))
        bad_ip = bytearray(wrap_v4(6, IP4_IMPAIR, IP4_GUEST, b"\x00" * 20))
        bad_ip[10] ^= 0xAA
        self._send(conn, ethernet(MAC_GUEST, IPV4, bytes(bad_ip), source=MAC_HOST))
        bad_tcp = bytearray(host_tcp4(IP4_IMPAIR, 80, 50000, 1, 1, PSH | ACK, b"corrupt"))
        bad_tcp[-1] ^= 0x55
        self._send(conn, bytes(bad_tcp))

    def _exchange(self, conn: socket.socket) -> None:
        tcp4: dict[tuple[bytes, int, int], dict] = {}
        tcp6: dict[tuple[bytes, int, int], dict] = {}
        syn_drop_done = False
        soak_count = 0
        try:
            while not self._stop.is_set():
                try:
                    frame = self._receive(conn)
                except ValueError:
                    if self._stop.is_set() or EXPECTED_STAGES.issubset(self._stages):
                        return
                    raise
                if len(frame) < 42 or frame[6:12] != MAC_GUEST:
                    continue
                eth = frame[12:14]
                if eth == ARP:
                    arp = frame[14:42]
                    op = struct.unpack_from("!H", arp, 6)[0]
                    spa, tpa = arp[14:18], arp[24:28]
                    if op == 1 and spa == IP4_GUEST and tpa in (IP4_HOST, IP4_IMPAIR, IP4_DNS):
                        rep = struct.pack("!HHBBH6s4s6s4s", 1, 0x0800, 6, 4, 2, MAC_HOST, tpa, MAC_GUEST, spa)
                        self._send(conn, ethernet(MAC_GUEST, ARP, rep, source=MAC_HOST))
                    continue

                if eth == ETH_IPV6:
                    ip = frame[14:]
                    plen = struct.unpack_from("!H", ip, 4)[0]
                    nh, hop = ip[6], ip[7]
                    src6, dst6 = ip[8:24], ip[24:40]
                    payload = ip[40:40 + plen]
                    if nh == 58 and hop == 255:
                        kind = payload[0]
                        if kind == 133:
                            self._send(conn, build_ra())
                        elif kind == 135:
                            target = payload[8:24]
                            if src6 != UNSPEC6 and target in (IP6_LL_HOST, IP6_DNS, IP6_PEER, IP6_HTTPS):
                                self._send(conn, build_na(target, src6, target))
                    elif nh == 58 and payload[0] == 128 and dst6 == IP6_PEER:
                        body = bytearray(payload)
                        body[0] = 129
                        struct.pack_into("!H", body, 2, 0)
                        cs = checksum(pseudo_v6(IP6_PEER, src6, 58, len(body)) + bytes(body))
                        struct.pack_into("!H", body, 2, cs)
                        self._send(conn, (MAC_GUEST + MAC_HOST + ETH_IPV6 + wrap_v6(58, 64, IP6_PEER, src6, bytes(body))).ljust(60, b"\0"))
                        self._mark("icmp6")
                    elif nh == 17 and dst6 == IP6_DNS:
                        sport, dport = struct.unpack_from("!HH", payload, 0)
                        if dport == 53:
                            qid, qn, qt = parse_dns(payload[8:])
                            ans = self._dns_answer(qid, qn, qt)
                            length = 8 + len(ans)
                            dg = struct.pack("!HHHH", 53, sport, length, 0) + ans
                            cs = checksum(pseudo_v6(IP6_DNS, src6, 17, length) + dg) or 0xFFFF
                            self._send(conn, (MAC_GUEST + MAC_HOST + ETH_IPV6 + wrap_v6(17, 64, IP6_DNS, src6, dg[:6] + struct.pack("!H", cs) + dg[8:])).ljust(60, b"\0"))
                    elif nh == 6 and dst6 == IP6_HTTPS:
                        self._handle_tls6(conn, tcp6, src6, dst6, payload)
                    continue

                if eth != IPV4:
                    continue
                total = struct.unpack_from("!H", frame, 16)[0]
                proto = frame[23]
                src4, dst4 = frame[26:30], frame[30:34]
                payload = frame[34:14 + total]
                if proto == 1 and payload[0] == 8 and dst4 == IP4_HOST:
                    rep = bytearray(payload)
                    rep[0] = 0
                    struct.pack_into("!H", rep, 2, 0)
                    struct.pack_into("!H", rep, 2, checksum(bytes(rep)))
                    self._send(conn, ethernet(MAC_GUEST, IPV4, wrap_v4(1, IP4_HOST, IP4_GUEST, bytes(rep)), source=MAC_HOST))
                    self._mark("icmp4")
                elif proto == 17 and dst4 == IP4_DNS:
                    sport, dport = struct.unpack_from("!HH", payload, 0)
                    if dport == 53:
                        qid, qn, qt = parse_dns(payload[8:])
                        self._send(conn, host_udp4(IP4_DNS, 53, sport, self._dns_answer(qid, qn, qt)))
                elif proto == 6:
                    sport, dport, seq, ack, off, flags = struct.unpack_from("!HHIIBB", payload, 0)
                    data = payload[(off >> 4) * 4:]
                    key = (dst4, dport, sport)
                    if flags == SYN:
                        if dst4 == IP4_IMPAIR and dport == 80 and not syn_drop_done:
                            syn_drop_done = True
                            self._inject_garbage(conn)
                            self._mark("syn_dropped")
                            continue  # Drop first SYN to force RTO retransmit!
                        gnext = advance(seq, 1)
                        hseq = 0x62000001
                        tcp4[key] = {"dst": dst4, "sport": sport, "dport": dport, "gnext": gnext, "hseq": hseq, "buf": bytearray(), "done": False}
                        self._send(conn, host_tcp4(dst4, dport, sport, hseq - 1, gnext, SYN | ACK))
                    elif key in tcp4:
                        c = tcp4[key]
                        if data:
                            c["gnext"] = advance(seq, len(data))
                            c["buf"].extend(data)
                            if c["dport"] == 80 and b"\r\n\r\n" in c["buf"] and not c["done"]:
                                c["done"] = True
                                raw_req = bytes(c["buf"])
                                if b"/reorder" in raw_req:
                                    hdrs = b"HTTP/1.1 200 OK\r\nContent-Length: 23\r\nConnection: close\r\n\r\n"
                                    body = b"impairment-reordered-ok"
                                    s0 = c["hseq"]
                                    s1 = advance(s0, len(hdrs))
                                    c["hseq"] = advance(s1, len(body) + 1)
                                    # Out-of-order: send body BEFORE headers + duplicate ACK + corrupt frame!
                                    self._send(conn, host_tcp4(c["dst"], 80, c["sport"], s0, c["gnext"], ACK))
                                    self._send(conn, host_tcp4(c["dst"], 80, c["sport"], s0, c["gnext"], ACK))
                                    self._send(conn, host_tcp4(c["dst"], 80, c["sport"], s1, c["gnext"], PSH | ACK | FIN, body))
                                    self._inject_garbage(conn)
                                    time.sleep(0.025)
                                    self._send(conn, host_tcp4(c["dst"], 80, c["sport"], s0, c["gnext"], PSH | ACK, hdrs))
                                    self._mark("reordered_http")
                                elif b"/after-restart" in raw_req:
                                    body = b"netd-restart-recovered"
                                    resp = f"HTTP/1.1 200 OK\r\nContent-Length: {len(body)}\r\nConnection: close\r\n\r\n".encode() + body
                                    hseq = c["hseq"]
                                    c["hseq"] = advance(hseq, len(resp) + 1)
                                    self._send(conn, host_tcp4(c["dst"], 80, c["sport"], hseq, c["gnext"], PSH | ACK | FIN, resp))
                                    self._mark("post_restart_http")
                            elif c["dport"] == 5432 and bytes(c["buf"]) == b"soak" and not c["done"]:
                                c["done"] = True
                                reply = b"soak-ack"
                                hseq = c["hseq"]
                                c["hseq"] = advance(hseq, len(reply) + 1)
                                self._send(conn, host_tcp4(c["dst"], 5432, c["sport"], hseq, c["gnext"], PSH | ACK | FIN, reply))
                                soak_count += 1
                                if soak_count >= 24:
                                    self._mark("soak_24")
                            elif c["dport"] == 5433 and bytes(c["buf"]) == b"arm-crash" and not c["done"]:
                                c["done"] = True
                                reply = b"armed"
                                hseq = c["hseq"]
                                c["hseq"] = advance(hseq, len(reply))
                                self._send(conn, host_tcp4(c["dst"], 5433, c["sport"], hseq, c["gnext"], PSH | ACK, reply))
                        if flags & FIN:
                            c["gnext"] = advance(c["gnext"], 1)
                            self._send(conn, host_tcp4(c["dst"], c["dport"], c["sport"], c["hseq"], c["gnext"], ACK))
        finally:
            self._done.set()

    def _dns_answer(self, qid: bytes, qn: bytes, qt: int) -> bytes:
        if qn == b"impair.du.net.test":
            return dns_rr(qid, qn, 1, IP4_IMPAIR) if qt == 1 else dns_err(qid, qn, qt, 3)
        if qn in (b"soak0.du.net.test", b"soak1.du.net.test"):
            return dns_rr(qid, qn, 1, IP4_IMPAIR) if qt == 1 else dns_err(qid, qn, qt, 3)
        if qn == b"https.du.net.test":
            return dns_rr(qid, qn, 28, IP6_HTTPS) if qt == 28 else dns_err(qid, qn, qt, 3)
        return dns_err(qid, qn, qt, 3)

    def _handle_tls6(self, conn: socket.socket, tcp6: dict, src6: bytes, dst6: bytes, payload: bytes) -> None:
        sport, dport, seq, ack, off, flags = struct.unpack_from("!HHIIBB", payload, 0)
        data = payload[(off >> 4) * 4:]
        key = (dst6, dport, sport)
        if flags == SYN and dport == 443:
            gnext = advance(seq, 1)
            hseq = 0x73000001
            ib, ob = ssl.MemoryBIO(), ssl.MemoryBIO()
            tcp6[key] = {"sport": sport, "dport": dport, "dst": dst6, "guest": src6, "gnext": gnext, "hseq": hseq, "buf": bytearray(), "done": False, "bio": (ib, ob, self._tls_ctx.wrap_bio(ib, ob, server_side=True))}
            self._send(conn, tcp6_frame(dst6, src6, dport, sport, hseq - 1, gnext, SYN | ACK))
        elif key in tcp6:
            c = tcp6[key]
            if data:
                c["gnext"] = advance(seq, len(data))
                ib, ob, tls = c["bio"]
                ib.write(data)
                try:
                    tls.do_handshake()
                except ssl.SSLWantReadError:
                    pass
                out = ob.read()
                if out:
                    self._send_v6(conn, c, out)
                while True:
                    try:
                        chunk = tls.read(4096)
                    except (ssl.SSLWantReadError, ssl.SSLEOFError):
                        break
                    if not chunk:
                        break
                    c["buf"].extend(chunk)
                if b"\r\n\r\n" in c["buf"] and not c["done"]:
                    c["done"] = True
                    body = b"tls13-under-impairment-ok"
                    tls.write(f"HTTP/1.1 200 OK\r\nContent-Length: {len(body)}\r\nConnection: close\r\n\r\n".encode() + body)
                    self._send_v6(conn, c, ob.read(), fin=True)
                    self._mark("tls13_impairment")
            if flags & FIN:
                c["gnext"] = advance(c["gnext"], 1)
                self._send(conn, tcp6_frame(c["dst"], c["guest"], c["dport"], c["sport"], c["hseq"], c["gnext"], ACK))

    def _send_v6(self, conn: socket.socket, c: dict, out: bytes, *, fin: bool = False) -> None:
        if not out and not fin:
            return
        for i in range(0, max(1, len(out)), 1400):
            seg = out[i:i + 1400]
            last = i + 1400 >= len(out)
            fl = ACK | (PSH if seg else 0) | (FIN if fin and last else 0)
            s = c["hseq"]
            c["hseq"] = advance(s, len(seg) + (1 if (fin and last) else 0))
            self._send(conn, tcp6_frame(c["dst"], c["guest"], c["dport"], c["sport"], s, c["gnext"], fl, seg))
