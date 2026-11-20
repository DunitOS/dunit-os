"""Isolated wire peer for outbound application sockets through stock netd."""
from __future__ import annotations

import socket
import struct
from threading import Event

from net_dhcp_transport import ARP, BROADCAST_MAC, IPV4, ipv4, parse_ipv4
from net_live_transport import ACK, FIN, PSH, SYN, advance, pseudo
from net_static_transport import arp, ethernet
from net_test_transport import EthernetPeer, GUEST_MAC, HOST_MAC, checksum

GUEST = bytes((192, 0, 2, 12))
HOST = bytes((192, 0, 2, 1))
DELAYED_HOST = bytes((192, 0, 2, 53))
UDP_PORT = 5433
TCP_PORT = 5432
UDP_REQUEST = b"fixture-udp-request-v1"
UDP_REPLY = b"fixture-udp-truncated-v1"
TCP_LENGTH = 5000
TCP_REPLY = b"fixture-tcp-partial-read-and-EOF"
CONFLICT_MAC = bytes.fromhex("020000000044")
RST = 0x04
STAGES = ("udp_datagram", "tcp_single_syn_after_delayed_arp", "tcp_reset_challenged_and_delivered", "tcp_zero_window_reopened", "tcp_stream",
          "tcp_reordered_reply_and_fin",
          "tcp_fin",
          "address_conflict_injected")


def host_ipv4(protocol: int, payload: bytes, *, remote: bytes = HOST) -> bytes:
    return ethernet(GUEST_MAC, HOST_MAC, IPV4, ipv4(protocol, remote, GUEST, payload))


def host_udp(target: int, data: bytes) -> bytes:
    length = 8 + len(data)
    datagram = struct.pack("!HHHH", UDP_PORT, target, length, 0) + data
    check = checksum(pseudo(HOST, GUEST, 17, length) + datagram) or 0xffff
    return host_ipv4(17, datagram[:6] + struct.pack("!H", check) + datagram[8:])


def host_tcp(target: int, seq: int, ack: int, flags: int, data: bytes = b"",
             *, window: int = 4096, remote: bytes = HOST) -> bytes:
    header = struct.pack("!HHIIBBHHH", TCP_PORT, target, seq, ack, 0x50, flags, window, 0, 0)
    segment = header + data
    check = checksum(pseudo(remote, GUEST, 6, len(segment)) + segment)
    return host_ipv4(6, segment[:16] + struct.pack("!H", check) + segment[18:], remote=remote)


def guest_transport(frame: bytes, protocol: int, *, remote: bytes = HOST) -> bytes:
    source, destination, payload = parse_ipv4(frame, protocol)
    if frame[:6] != HOST_MAC or source != GUEST or destination != remote:
        raise ValueError("unexpected destination or source in application socket frame")
    return payload


def check_udp(frame: bytes) -> int:
    datagram = guest_transport(frame, 17)
    if len(datagram) < 8:
        raise ValueError("short application UDP datagram")
    source, destination, length, check = struct.unpack("!HHHH", datagram[:8])
    if (source < 49152 or destination != UDP_PORT or length != len(datagram) or not check or
            checksum(pseudo(GUEST, HOST, 17, length) + datagram) or datagram[8:] != UDP_REQUEST):
        raise ValueError("unexpected application UDP tuple, payload or checksum")
    return source


def parse_tcp(frame: bytes, *, remote: bytes = HOST) -> tuple[int, int, int, int, bytes]:
    segment = guest_transport(frame, 6, remote=remote)
    if len(segment) < 20:
        raise ValueError("short TCP segment")
    source, destination, seq, ack, offset, flags, _, _, urgent = struct.unpack(
        "!HHIIBBHHH", segment[:20])
    size = (offset >> 4) * 4
    if (source < 49152 or destination != TCP_PORT or offset & 15 or urgent or
            not 20 <= size <= len(segment) or
            checksum(pseudo(GUEST, remote, 6, len(segment)) + segment)):
        raise ValueError("unexpected application TCP tuple or checksum")
    return source, seq, ack, flags, segment[size:]


class NetSocketPeer(EthernetPeer):
    """Require intact datagrams and stream bytes on the physical E1000 path."""

    def __init__(self, workdir):
        super().__init__(workdir)
        self._done = Event()
        self._stages: list[str] = []
        self._probes = 0
        self._announcements = 0
        self._tcp_received = 0
        self._delayed_arp_requests = 0

    def wait_complete(self, timeout: float) -> bool:
        return self._done.wait(timeout) and tuple(self._stages) == STAGES

    def _exchange(self, connection: socket.socket) -> None:
        try:
            self._wire(connection)
        finally:
            self._done.set()
        # Keep carrier and the registered service alive through the final
        # guest assertions. The harness closes the isolated connection.
        self._stop.wait()

    def _next_ipv4(self, connection: socket.socket) -> bytes:
        for _ in range(12):
            frame = self._receive(connection)
            if frame[12:14] == IPV4:
                if self._probes != 3 or self._announcements == 0:
                    raise ValueError("socket transmitted before static address probing")
                return frame
            if frame[12:14] != ARP:
                raise ValueError("unexpected EtherType from socket fixture")
            if frame == ethernet(BROADCAST_MAC, GUEST_MAC, ARP,
                                 arp(1, GUEST_MAC, bytes(4), bytes(6), GUEST)):
                self._probes += 1
                if self._probes > 3 or self._announcements:
                    raise ValueError("incorrect static probe ordering")
            elif frame == ethernet(BROADCAST_MAC, GUEST_MAC, ARP,
                                   arp(1, GUEST_MAC, GUEST, bytes(6), GUEST)):
                self._announcements += 1
                if self._announcements > 2 or self._probes != 3:
                    raise ValueError("incorrect static announcement ordering")
            elif frame == ethernet(BROADCAST_MAC, GUEST_MAC, ARP,
                                   arp(1, GUEST_MAC, GUEST, bytes(6), HOST)):
                self._send(connection, ethernet(GUEST_MAC, HOST_MAC, ARP,
                                                 arp(2, HOST_MAC, HOST, GUEST_MAC, GUEST)))
            elif frame == ethernet(BROADCAST_MAC, GUEST_MAC, ARP,
                                   arp(1, GUEST_MAC, GUEST, bytes(6), DELAYED_HOST)):
                self._delayed_arp_requests += 1
                if self._delayed_arp_requests == 3:
                    self._send(connection, ethernet(GUEST_MAC, HOST_MAC, ARP,
                                                     arp(2, HOST_MAC, DELAYED_HOST, GUEST_MAC, GUEST)))
                elif self._delayed_arp_requests > 3:
                    raise ValueError("resolved TCP destination kept requesting ARP")
            else:
                raise ValueError("unexpected ARP from socket fixture")
        raise ValueError("no IPv4 traffic within ARP frame budget")

    def _wire(self, connection: socket.socket) -> None:
        endpoint = check_udp(self._next_ipv4(connection))
        self._send(connection, host_udp(endpoint, UDP_REPLY))
        self._stages.append(STAGES[0])
        self._check_reset(connection)

        endpoint, seq, ack, flags, data = parse_tcp(self._next_ipv4(connection))
        if (ack, flags, data) != (0, SYN, b""):
            raise ValueError("expected TCP SYN from application socket")
        next_guest = advance(seq, 1)
        host_next = 0x50900001
        self._send(connection, host_tcp(endpoint, host_next - 1, next_guest, SYN | ACK, window=0))
        if parse_tcp(self._next_ipv4(connection)) != (endpoint, next_guest, host_next, ACK, b""):
            raise ValueError("missing ACK for zero-window SYN-ACK")
        probe = (endpoint, next_guest, host_next, ACK | PSH, b"\0")
        if parse_tcp(self._next_ipv4(connection)) != probe:
            raise ValueError("expected a one-byte zero-window probe")
        self._send(connection, host_tcp(endpoint, host_next, next_guest, ACK, window=0))
        self._send(connection, host_tcp(endpoint, host_next, next_guest, ACK))
        if parse_tcp(self._next_ipv4(connection)) != probe:
            raise ValueError("reopening the window did not retransmit the probe")
        next_guest = advance(next_guest, 1)
        self._tcp_received = 1
        self._send(connection, host_tcp(endpoint, host_next, next_guest, ACK))
        self._stages.append("tcp_zero_window_reopened")

        for _ in range(64):
            source, seq, ack, flags, data = parse_tcp(self._next_ipv4(connection))
            if (source != endpoint or seq != next_guest or ack != host_next or
                    flags not in (ACK, PSH | ACK)):
                raise ValueError("unexpected guest TCP transmit")
            if not data:
                continue
            first = self._tcp_received
            self._tcp_received += len(data)
            if self._tcp_received > TCP_LENGTH or data != bytes(
                    index % 251 for index in range(first, self._tcp_received)):
                raise ValueError("guest TCP stream was truncated or changed")
            next_guest = advance(next_guest, len(data))
            self._send(connection, host_tcp(endpoint, host_next, next_guest, ACK))
            if self._tcp_received == TCP_LENGTH:
                break
        else:
            raise ValueError("guest TCP stream did not complete")
        self._stages.append("tcp_stream")

        first, middle, last = TCP_REPLY[:9], TCP_REPLY[9:18], TCP_REPLY[18:]
        middle_start = advance(host_next, len(first))
        last_start = advance(middle_start, len(middle))
        tail = host_tcp(endpoint, last_start, next_guest, PSH | ACK | FIN, last)
        body = host_tcp(endpoint, middle_start, next_guest, PSH | ACK, middle)
        gap_ack = (endpoint, next_guest, host_next, ACK, b"")
        for frame in (tail, body, tail):
            self._send(connection, frame)
            if parse_tcp(self._next_ipv4(connection)) != gap_ack:
                raise ValueError("TCP acknowledged a gap before receiving its missing prefix")
        self._send(connection, host_tcp(endpoint, host_next, next_guest, PSH | ACK, first))
        host_next = advance(last_start, len(last) + 1)

        for _ in range(12):
            source, seq, ack, flags, data = parse_tcp(self._next_ipv4(connection))
            if (source != endpoint or seq != next_guest or data or
                    flags not in (ACK, FIN | ACK) or ack != host_next):
                raise ValueError("unexpected guest TCP response during half-close")
            if flags == (FIN | ACK):
                if ack != host_next:
                    raise ValueError("guest FIN did not acknowledge peer EOF")
                self._send(connection, host_tcp(endpoint, host_next, advance(next_guest, 1), ACK))
                self._stages.append("tcp_reordered_reply_and_fin")
                self._stages.append("tcp_fin")
                break
        else:
            raise ValueError("application TCP socket never closed its write side")

        # A fast guest may finish before the second scheduled announcement.
        # At that point the next frame must be that announcement, not extra
        # socket traffic.
        if self._announcements == 1:
            second = ethernet(BROADCAST_MAC, GUEST_MAC, ARP,
                              arp(1, GUEST_MAC, GUEST, bytes(6), GUEST))
            if self._receive(connection) != second:
                raise ValueError("missing second static address announcement")
            self._announcements += 1

        # A different station claims the configured address. Stock netd must
        # withdraw it and close its service bridge generation.
        self._send(connection, ethernet(BROADCAST_MAC, CONFLICT_MAC, ARP,
                                         arp(1, CONFLICT_MAC, GUEST, bytes(6), GUEST)))
        self._stages.append("address_conflict_injected")

    def _check_reset(self, connection: socket.socket) -> None:
        def receive():
            return parse_tcp(self._next_ipv4(connection), remote=DELAYED_HOST)

        def send(packet_seq, packet_ack, packet_flags):
            self._send(connection, host_tcp(endpoint, packet_seq, packet_ack, packet_flags,
                                             remote=DELAYED_HOST))

        endpoint, sequence, ack, flags, data = receive()
        if (ack, flags, data) != (0, SYN, b"") or self._delayed_arp_requests != 3:
            raise ValueError("expected TCP SYN for reset test")
        guest_next = advance(sequence, 1)
        host_next = 0x50800001
        send(host_next - 1, guest_next, SYN | ACK)
        first = receive()
        if first == (endpoint, guest_next, host_next, ACK, b""):
            first = receive()
        if first != (endpoint, guest_next, host_next, PSH | ACK, b"r"):
            raise ValueError("reset test never reached established data transfer")
        self._stages.append("tcp_single_syn_after_delayed_arp")
        guest_next = advance(guest_next, 1)
        send(host_next, guest_next, ACK)
        send(host_next + 1, 0, RST)
        if receive() != (endpoint, guest_next, host_next, ACK, b""):
            raise ValueError("missing challenge ACK for in-window RST with a wrong sequence")
        send(host_next, 0, RST)
        self._stages.append("tcp_reset_challenged_and_delivered")

    def _finish_exchange(self) -> dict:
        if tuple(self._stages) != STAGES or self._announcements != 2:
            self.errors.append(f"incomplete socket exchange: {self._stages!r}")
        return {
            "protocols": ["ARP", "UDP", "TCP"],
            "socket_stages": self._stages,
            "tcp_bytes": self._tcp_received,
            "tcp_delayed_arp_requests": self._delayed_arp_requests,
            "udp_reply_bytes": len(UDP_REPLY),
        }
