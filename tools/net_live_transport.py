"""Strict localhost Ethernet peer for the supervised native packet queue fixture."""
from __future__ import annotations

from pathlib import Path
import socket
import struct

from net_test_transport import GUEST_MAC, HOST_MAC, EthernetPeer, checksum

GUEST = bytes((192, 0, 2, 2))
HOST = bytes((192, 0, 2, 1))
ARP = b"\x08\x06"
IPV4 = b"\x08\x00"
ETH_BROADCAST = b"\xff" * 6
DNS_QUESTION = b"\x02du\x03net\x04test\x00\x00\x01\x00\x01"
TCP_QUESTION = b"\x03tcp" + DNS_QUESTION
TCP_PORT = 49153
ECHO_DATA = b"dunit-netd-live"


def ether(kind: bytes, payload: bytes) -> bytes:
    return (GUEST_MAC + HOST_MAC + kind + payload).ljust(60, b"\0")


def ipv4(protocol: int, payload: bytes) -> bytes:
    header = struct.pack("!BBHHHBBH4s4s", 0x45, 0, 20 + len(payload), 0,
                         0x4000, 64, protocol, 0, HOST, GUEST)
    header = header[:10] + struct.pack("!H", checksum(header)) + header[12:]
    return ether(IPV4, header + payload)


def guest_ipv4(frame: bytes, protocol: int) -> bytes:
    if frame[:14] != HOST_MAC + GUEST_MAC + IPV4:
        raise ValueError("expected unicast guest IPv4")
    header = frame[14:34]
    total = int.from_bytes(header[2:4], "big")
    if (header[0] != 0x45 or not 20 <= total <= len(frame) - 14 or
            header[9] != protocol or header[12:20] != GUEST + HOST or checksum(header)):
        raise ValueError("invalid guest IPv4 header")
    return frame[34:14 + total]


def echo_response(frame: bytes) -> tuple[bytes, int]:
    echo = guest_ipv4(frame, 1)
    if echo[:2] != b"\x08\x00" or echo[6:8] != b"\x00\x01" or echo[8:] != ECHO_DATA or checksum(echo):
        raise ValueError("invalid guest ICMP echo")
    reply = b"\0\0\0\0" + echo[4:]
    reply = reply[:2] + struct.pack("!H", checksum(reply)) + reply[4:]
    return ipv4(1, reply), int.from_bytes(echo[4:6], "big")


def pseudo(source: bytes, destination: bytes, protocol: int, length: int) -> bytes:
    return struct.pack("!4s4sBBH", source, destination, 0, protocol, length)


def dns_answer(dns: bytes, question: bytes, answer_octet: int, truncated: bool) -> bytes:
    if dns[2:12] != struct.pack("!5H", 0x0100, 1, 0, 0, 0) or dns[12:] != question:
        raise ValueError("invalid guest DNS question")
    flags = 0x8380 if truncated else 0x8180
    wire = dns[:2] + struct.pack("!5H", flags, 1, int(not truncated), 0, 0) + question
    if truncated:
        return wire
    return wire + struct.pack("!HHHIH4B", 0xc00c, 1, 1, 30, 4, 192, 0, 2, answer_octet)


def dns_response(frame: bytes, answer_octet: int, *, truncated: bool = False) -> bytes:
    udp = guest_ipv4(frame, 17)
    if len(udp) < 8:
        raise ValueError("truncated UDP header")
    source, destination, length, check = struct.unpack("!HHHH", udp[:8])
    if (not source or destination != 53 or length != len(udp) or not check or
            checksum(pseudo(GUEST, HOST, 17, length) + udp)):
        raise ValueError("invalid DNS transport")
    question = TCP_QUESTION if truncated else DNS_QUESTION
    dns_reply = dns_answer(udp[8:], question, answer_octet, truncated)
    udp_length = 8 + len(dns_reply)
    reply = struct.pack("!HHHH", 53, source, udp_length, 0) + dns_reply
    udp_checksum = checksum(pseudo(HOST, GUEST, 17, udp_length) + reply) or 0xffff
    return ipv4(17, reply[:6] + struct.pack("!H", udp_checksum) + reply[8:])


def guest_tcp(frame: bytes) -> tuple[int, int, int, bytes]:
    segment = guest_ipv4(frame, 6)
    if len(segment) < 20:
        raise ValueError("truncated TCP header")
    source, destination, seq, ack, offset, flags, _, _, urgent = struct.unpack(
        "!HHIIBBHHH", segment[:20])
    size = (offset >> 4) * 4
    if (source != TCP_PORT or destination != 53 or offset & 15 or urgent or
            not 20 <= size <= len(segment) or
            checksum(pseudo(GUEST, HOST, 6, len(segment)) + segment)):
        raise ValueError("invalid guest TCP segment")
    return seq, ack, flags, segment[size:]


def host_tcp(seq: int, ack: int, flags: int, data: bytes = b"") -> bytes:
    header = struct.pack("!HHIIBBHHH", 53, TCP_PORT, seq, ack, 0x50, flags, 4096, 0, 0)
    segment = header + data
    check = checksum(pseudo(HOST, GUEST, 6, len(segment)) + segment)
    return ipv4(6, segment[:16] + struct.pack("!H", check) + segment[18:])


class NetLivePeer(EthernetPeer):
    """Verify two complete protocol exchanges across the supervised restart."""

    def __init__(self, workdir: Path):
        super().__init__(workdir)
        self._identifiers: list[int] = []
        self._tcp_rounds = 0

    def _exchange(self, connection: socket.socket) -> None:
        for answer_octet in (3, 4):
            frame = self._receive(connection)
            arp_prefix = struct.pack("!HHBB", 1, 0x0800, 6, 4)
            request = (ETH_BROADCAST + GUEST_MAC + ARP + arp_prefix + b"\0\x01" +
                       GUEST_MAC + GUEST + b"\0" * 6 + HOST).ljust(60, b"\0")
            if frame != request:
                raise ValueError("invalid guest ARP request")
            self._send(connection, ether(ARP, arp_prefix + b"\0\x02" +
                                        HOST_MAC + HOST + GUEST_MAC + GUEST))
            reply, identifier = echo_response(self._receive(connection))
            if identifier in self._identifiers:
                raise ValueError("ICMP exchange did not move to a fresh service process")
            self._identifiers.append(identifier)
            self._send(connection, reply)
            self._send(connection, dns_response(self._receive(connection), answer_octet))
            self._send(connection, dns_response(self._receive(connection), answer_octet, truncated=True))
            self._tcp_dns(connection, answer_octet)

    def _tcp_dns(self, connection: socket.socket, answer_octet: int) -> None:
        first = self._receive(connection)  # Deliberately lose the first SYN.
        guest_seq, ack, flags, data = guest_tcp(first)
        if (ack, flags, data) != (0, 2, b"") or self._receive(connection) != first:
            raise ValueError("missing identical retransmit after dropped TCP SYN")
        guest_next = guest_seq + 1
        host_seq = 0x32000000 + answer_octet
        self._send(connection, host_tcp(host_seq, guest_next, 0x12))
        host_seq += 1

        seq, ack, flags, data = guest_tcp(self._receive(connection))
        if ((seq, ack, flags) != (guest_next, host_seq, 0x18) or
                len(data) < 2 or int.from_bytes(data[:2], "big") != len(data) - 2):
            raise ValueError("invalid TCP DNS query or SYN acknowledgement")
        guest_next += len(data)
        response = dns_answer(data[2:], TCP_QUESTION, answer_octet, False)
        framed = struct.pack("!H", len(response)) + response
        self._send(connection, host_tcp(host_seq, guest_next, 0x18, framed[:1]))
        host_seq += 1
        if guest_tcp(self._receive(connection)) != (guest_next, host_seq, 0x10, b""):
            raise ValueError("client did not acknowledge first TCP length byte")

        self._send(connection, host_tcp(host_seq, guest_next, 0x19, framed[1:]))
        host_seq += len(framed)
        if guest_tcp(self._receive(connection)) != (guest_next, host_seq, 0x11, b""):
            raise ValueError("client did not finish its half of the TCP stream")
        self._send(connection, host_tcp(host_seq, guest_next + 1, 0x10))
        self._tcp_rounds += 1

    def _finish_exchange(self) -> dict:
        if (self._packets, self._responses, len(self._identifiers), self._tcp_rounds) != (18, 16, 2, 2):
            self.errors.append("incomplete ARP/ICMP/UDP/TCP DNS and restart exchange")
        return {"protocols": ["ARP", "ICMPv4", "UDP", "TCP", "DNS"],
                "guest_identifiers": self._identifiers, "tcp_rounds": self._tcp_rounds}
