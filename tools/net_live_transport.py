"""Strict localhost Ethernet peer for the supervised native packet queue fixture."""
from __future__ import annotations

from pathlib import Path
import socket
import struct

from net_test_transport import GUEST_MAC, HOST_MAC, NetPeer

GUEST = bytes((192, 0, 2, 2))
HOST = bytes((192, 0, 2, 1))
ARP = b"\x08\x06"
IPV4 = b"\x08\x00"
ETH_BROADCAST = b"\xff" * 6
DNS_QUESTION = b"\x02du\x03net\x04test\x00\x00\x01\x00\x01"
ECHO_DATA = b"dunit-netd-live"


def checksum(data: bytes) -> int:
    if len(data) % 2:
        data += b"\0"
    total = sum(int.from_bytes(data[i:i+2], "big") for i in range(0, len(data), 2))
    while total > 0xffff:
        total = (total & 0xffff) + (total >> 16)
    return (~total) & 0xffff


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


def dns_response(frame: bytes, answer_octet: int) -> bytes:
    udp = guest_ipv4(frame, 17)
    if len(udp) < 8:
        raise ValueError("truncated UDP header")
    source, destination, length, check = struct.unpack("!HHHH", udp[:8])
    pseudo = lambda sender, dest, size: struct.pack("!4s4sBBH", sender, dest, 0, 17, size)
    if (not source or destination != 53 or length != len(udp) or not check or
            checksum(pseudo(GUEST, HOST, length) + udp)):
        raise ValueError("invalid DNS transport")
    dns = udp[8:]
    if dns[2:12] != struct.pack("!5H", 0x0100, 1, 0, 0, 0) or dns[12:] != DNS_QUESTION:
        raise ValueError("invalid guest DNS question")
    dns_reply = (dns[:2] + struct.pack("!5H", 0x8180, 1, 1, 0, 0) + DNS_QUESTION +
                 struct.pack("!HHHIH4B", 0xc00c, 1, 1, 30, 4, 192, 0, 2, answer_octet))
    udp_length = 8 + len(dns_reply)
    reply = struct.pack("!HHHH", 53, source, udp_length, 0) + dns_reply
    udp_checksum = checksum(pseudo(HOST, GUEST, udp_length) + reply) or 0xffff
    return ipv4(17, reply[:6] + struct.pack("!H", udp_checksum) + reply[8:])


class NetLivePeer(NetPeer):
    """Verify two complete protocol exchanges across the supervised restart."""

    def __init__(self, workdir: Path):
        super().__init__(workdir)
        self._identifiers: list[int] = []

    def _receive(self, connection: socket.socket) -> bytes:
        header = self._read(connection, 4)
        if header is None:
            raise ValueError("missing Ethernet length prefix")
        length, = struct.unpack("!I", header)
        if not 60 <= length <= 1514:
            raise ValueError(f"invalid Ethernet frame length {length}")
        frame = self._read(connection, length)
        if frame is None:
            raise ValueError("missing Ethernet payload")
        self._packets += 1
        self._bytes += length
        return frame

    def _send(self, connection: socket.socket, frame: bytes) -> None:
        connection.sendall(struct.pack("!I", len(frame)) + frame)
        self._responses += 1

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

    def _finish_exchange(self) -> dict:
        if self._packets != 6 or self._responses != 6 or len(self._identifiers) != 2:
            self.errors.append("incomplete ARP/ICMP/UDP DNS and restart exchange")
        return {"protocols": ["ARP", "ICMPv4", "UDP", "DNS"], "guest_identifiers": self._identifiers}
