"""Isolated E1000 peer for application DNS resolution through stock netd."""
from __future__ import annotations

import socket
import struct
from threading import Event

from net_dhcp_transport import ARP, BROADCAST_MAC, IPV4, ipv4, parse_ipv4
from net_live_transport import ACK, FIN, PSH, SYN, advance, dns_answer, pseudo
from net_static_transport import arp, ethernet
from net_test_transport import EthernetPeer, GUEST_MAC, HOST_MAC, checksum

GUEST = bytes((192, 0, 2, 12))
PRIMARY_DNS = bytes((192, 0, 2, 1))
SECONDARY_DNS = bytes((192, 0, 2, 53))
TARGET_HOST = bytes((192, 0, 2, 10))
CONFLICT_MAC = bytes.fromhex("020000000044")

DNS_PORT = 53
TCP_PORT = 5432

Q_DU = b"\x02du\x03net\x04test\x00\x00\x01\x00\x01"
Q_MISSING = b"\x07missing" + Q_DU
Q_TCP = b"\x03tcp" + Q_DU
Q_FAILOVER = b"\x08failover" + Q_DU


def host_ipv4(protocol: int, payload: bytes, *, source: bytes) -> bytes:
    return ethernet(GUEST_MAC, HOST_MAC, IPV4, ipv4(protocol, source, GUEST, payload))


def host_udp(source_ip: bytes, src_port: int, dst_port: int, data: bytes) -> bytes:
    length = 8 + len(data)
    dgram = struct.pack("!HHHH", src_port, dst_port, length, 0) + data
    chk = checksum(pseudo(source_ip, GUEST, 17, length) + dgram) or 0xFFFF
    return host_ipv4(17, dgram[:6] + struct.pack("!H", chk) + dgram[8:], source=source_ip)


def host_tcp(source_ip: bytes, src_port: int, dst_port: int, seq: int, ack: int, flags: int, data: bytes = b"") -> bytes:
    hdr = struct.pack("!HHIIBBHHH", src_port, dst_port, seq, ack, 0x50, flags, 4096, 0, 0)
    seg = hdr + data
    chk = checksum(pseudo(source_ip, GUEST, 6, len(seg)) + seg)
    return host_ipv4(6, seg[:16] + struct.pack("!H", chk) + seg[18:], source=source_ip)


def parse_udp(frame: bytes) -> tuple[bytes, int, int, bytes]:
    src, dst, payload = parse_ipv4(frame, 17)
    if frame[:6] != HOST_MAC or src != GUEST or len(payload) < 8:
        raise ValueError("unexpected UDP frame")
    sport, dport, length, chk = struct.unpack("!HHHH", payload[:8])
    if length != len(payload) or not chk or checksum(pseudo(GUEST, dst, 17, length) + payload):
        raise ValueError("bad UDP checksum/length")
    return dst, sport, dport, payload[8:]


def parse_tcp(frame: bytes) -> tuple[bytes, int, int, int, int, int, bytes]:
    src, dst, seg = parse_ipv4(frame, 6)
    if frame[:6] != HOST_MAC or src != GUEST or len(seg) < 20:
        raise ValueError("unexpected TCP frame")
    sport, dport, seq, ack, off, flags, _, _, _ = struct.unpack("!HHIIBBHHH", seg[:20])
    size = (off >> 4) * 4
    if off & 15 or not 20 <= size <= len(seg) or checksum(pseudo(GUEST, dst, 6, len(seg)) + seg):
        raise ValueError("bad TCP checksum/header")
    return dst, sport, dport, seq, ack, flags, seg[size:]


def nxdomain_soa(query: bytes, qname: bytes) -> bytes:
    zone = 0xC014
    soa = struct.pack("!HHHIHHH5I", zone, 6, 1, 30, 24, zone, zone, 1, 60, 60, 60, 10)
    return query[:2] + struct.pack("!5H", 0x8183, 1, 0, 1, 0) + qname + soa


class NetDnsPeer(EthernetPeer):
    """Serves UDP DNS, negative SOA, TCP fallback, secondary failover, and echo."""

    def __init__(self, workdir, *, posix: bool = False):
        super().__init__(workdir)
        self._posix = posix
        self._done = Event()
        self._stages: list[str] = []
        self._du_queries = 0
        self._missing_queries = 0
        self._probes = 0
        self._announcements = 0
        self._tcp_dns_state = None
        self._tcp_echo_state = None

    def wait_complete(self, timeout: float) -> bool:
        expected = (
            ("du_udp", "missing_nxdomain", "tcp_echo", "conflict_injected")
            if self._posix
            else ("du_udp", "missing_nxdomain", "tcp_fallback", "secondary_failover", "tcp_echo", "conflict_injected")
        )
        return self._done.wait(timeout) and all(s in self._stages for s in expected)

    def _exchange(self, connection: socket.socket) -> None:
        try:
            self._serve(connection)
        finally:
            self._done.set()
        self._stop.wait()

    def _serve(self, connection: socket.socket) -> None:
        while True:
            frame = self._receive(connection)
            eth = frame[12:14]
            if eth == ARP:
                if frame == ethernet(BROADCAST_MAC, GUEST_MAC, ARP, arp(1, GUEST_MAC, bytes(4), bytes(6), GUEST)):
                    self._probes += 1
                    continue
                if frame == ethernet(BROADCAST_MAC, GUEST_MAC, ARP, arp(1, GUEST_MAC, GUEST, bytes(6), GUEST)):
                    self._announcements += 1
                    continue
                for target in (PRIMARY_DNS, SECONDARY_DNS, TARGET_HOST):
                    if frame == ethernet(BROADCAST_MAC, GUEST_MAC, ARP, arp(1, GUEST_MAC, GUEST, bytes(6), target)):
                        self._send(connection, ethernet(GUEST_MAC, HOST_MAC, ARP, arp(2, HOST_MAC, target, GUEST_MAC, GUEST)))
                        break
                continue

            if eth != IPV4:
                continue
            proto = frame[23]
            if proto == 17:
                dst, sport, dport, dns = parse_udp(frame)
                if dport != DNS_PORT:
                    raise ValueError(f"unexpected UDP dport={dport}")
                q = dns[12:]
                if dst == PRIMARY_DNS and q == Q_DU:
                    self._du_queries += 1
                    if self._du_queries > 1:
                        raise ValueError("du.net.test queried twice despite cache")
                    self._send(connection, host_udp(PRIMARY_DNS, DNS_PORT, sport, dns_answer(dns, Q_DU, 10, False)))
                    self._stages.append("du_udp")
                elif dst == PRIMARY_DNS and q == Q_MISSING:
                    self._missing_queries += 1
                    if self._missing_queries > 1:
                        raise ValueError("missing.du.net.test queried twice despite negative cache")
                    self._send(connection, host_udp(PRIMARY_DNS, DNS_PORT, sport, nxdomain_soa(dns, Q_MISSING)))
                    self._stages.append("missing_nxdomain")
                elif dst == PRIMARY_DNS and q == Q_TCP:
                    self._send(connection, host_udp(PRIMARY_DNS, DNS_PORT, sport, dns_answer(dns, Q_TCP, 20, True)))
                elif dst == PRIMARY_DNS and q == Q_FAILOVER:
                    pass  # Drop primary query to trigger secondary failover.
                elif dst == SECONDARY_DNS and q == Q_FAILOVER:
                    self._send(connection, host_udp(SECONDARY_DNS, DNS_PORT, sport, dns_answer(dns, Q_FAILOVER, 30, False)))
                    self._stages.append("secondary_failover")
                else:
                    raise ValueError(f"unexpected DNS query dst={dst!r} q={q!r}")
                continue

            if proto == 6:
                dst, sport, dport, seq, ack, flags, data = parse_tcp(frame)
                if dst == PRIMARY_DNS and dport == DNS_PORT:
                    if flags == SYN:
                        guest_next = advance(seq, 1)
                        host_next = 0x60000001
                        self._tcp_dns_state = [sport, guest_next, host_next, bytearray()]
                        self._send(connection, host_tcp(PRIMARY_DNS, DNS_PORT, sport, host_next - 1, guest_next, SYN | ACK))
                    elif self._tcp_dns_state and sport == self._tcp_dns_state[0]:
                        if data:
                            self._tcp_dns_state[1] = advance(self._tcp_dns_state[1], len(data))
                            self._tcp_dns_state[3].extend(data)
                            buf = bytes(self._tcp_dns_state[3])
                            if len(buf) >= 2:
                                qlen = struct.unpack("!H", buf[:2])[0]
                                if len(buf) == 2 + qlen:
                                    ans = dns_answer(buf[2:], Q_TCP, 20, False)
                                    wire = struct.pack("!H", len(ans)) + ans
                                    hseq = self._tcp_dns_state[2]
                                    self._tcp_dns_state[2] = advance(hseq, len(wire))
                                    self._send(connection, host_tcp(PRIMARY_DNS, DNS_PORT, sport, hseq, self._tcp_dns_state[1], PSH | ACK, wire))
                                    self._stages.append("tcp_fallback")
                        if flags & FIN:
                            self._tcp_dns_state[1] = advance(self._tcp_dns_state[1], 1)
                            self._send(connection, host_tcp(PRIMARY_DNS, DNS_PORT, sport, self._tcp_dns_state[2], self._tcp_dns_state[1], ACK))
                    continue

                if dst == TARGET_HOST and dport == TCP_PORT:
                    if flags == SYN:
                        guest_next = advance(seq, 1)
                        host_next = 0x70000001
                        self._tcp_echo_state = [sport, guest_next, host_next]
                        self._send(connection, host_tcp(TARGET_HOST, TCP_PORT, sport, host_next - 1, guest_next, SYN | ACK))
                    elif self._tcp_echo_state and sport == self._tcp_echo_state[0]:
                        if data == b"dns-tcp-ok":
                            self._tcp_echo_state[1] = advance(self._tcp_echo_state[1], len(data))
                            hseq = self._tcp_echo_state[2]
                            reply = b"dns-tcp-ack"
                            self._tcp_echo_state[2] = advance(hseq, len(reply) + 1)
                            self._send(connection, host_tcp(TARGET_HOST, TCP_PORT, sport, hseq, self._tcp_echo_state[1], PSH | ACK | FIN, reply))
                        elif flags & FIN:
                            self._tcp_echo_state[1] = advance(self._tcp_echo_state[1], 1)
                            self._send(connection, host_tcp(TARGET_HOST, TCP_PORT, sport, self._tcp_echo_state[2], self._tcp_echo_state[1], ACK))
                            self._stages.append("tcp_echo")
                            self._send(connection, ethernet(BROADCAST_MAC, CONFLICT_MAC, ARP, arp(1, CONFLICT_MAC, GUEST, bytes(6), GUEST)))
                            self._stages.append("conflict_injected")
                            return
                    continue

    def _finish_exchange(self) -> dict:
        return {
            "protocols": ["ARP", "UDP-DNS", "TCP-DNS", "TCP"],
            "stages": self._stages,
            "du_queries": self._du_queries,
            "missing_queries": self._missing_queries,
        }

