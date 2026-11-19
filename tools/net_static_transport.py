"""Isolated IPv4 wire verifier for static-address production netd."""
from __future__ import annotations

from select import select
import socket
import struct

from net_dhcp_transport import ARP, ARP_HEADER, BROADCAST_MAC, IPV4, ZERO, ipv4, parse_ipv4
from net_test_transport import GUEST_MAC, HOST_MAC, EthernetPeer, checksum

STATIC = bytes((192, 0, 2, 12))
LOCAL = bytes((192, 0, 2, 42))
GATEWAY = bytes((192, 0, 2, 1))
REMOTE = bytes((198, 51, 100, 7))
GATEWAY_MAC = bytes.fromhex("020000000022")
CONFLICT_MAC = bytes.fromhex("020000000033")
QUIET_SECONDS = 1.0
STAGES = (
    "three_probes_two_announcements",
    "direct_arp_and_icmp",
    "off_link_icmp_via_configured_gateway",
    "active_conflict_withdrawn",
)


def ethernet(destination: bytes, source: bytes, kind: bytes, payload: bytes) -> bytes:
    return (destination + source + kind + payload).ljust(60, b"\0")


def arp(operation: int, sender_mac: bytes, sender: bytes,
        target_mac: bytes, target: bytes) -> bytes:
    return (ARP_HEADER + struct.pack("!H", operation) +
            sender_mac + sender + target_mac + target)


def echo(kind: int, sequence: int) -> bytes:
    request = struct.pack("!BBHHH", kind, 0, 0, 0x7157, sequence) + b"dunit-static-route"
    return request[:2] + struct.pack("!H", checksum(request)) + request[4:]


class NetStaticPeer(EthernetPeer):
    """Require exact wire bytes, including the configured next-hop MAC."""

    def __init__(self, workdir):
        super().__init__(workdir)
        self._stages: list[str] = []

    def wait_complete(self, timeout: float) -> bool:
        self._thread.join(timeout=timeout)
        return tuple(self._stages) == STAGES and not self._thread.is_alive()

    def _arp_claim(self, connection: socket.socket, sender: bytes, phase: str) -> None:
        expected = ethernet(BROADCAST_MAC, GUEST_MAC, ARP,
                            arp(1, GUEST_MAC, sender, bytes(6), STATIC))
        if self._receive(connection) != expected:
            raise ValueError(f"invalid static IPv4 {phase}")

    def _who_has_static(self, connection: socket.socket) -> None:
        self._send(connection, ethernet(BROADCAST_MAC, HOST_MAC, ARP,
                                         arp(1, HOST_MAC, LOCAL, bytes(6), STATIC)))

    def _send_echo(self, connection: socket.socket, source: bytes, sequence: int) -> None:
        self._send(connection, ethernet(GUEST_MAC, HOST_MAC, IPV4,
                                         ipv4(1, source, STATIC, echo(8, sequence))))

    def _expect_echo(self, connection: socket.socket, destination: bytes,
                     mac: bytes, sequence: int) -> None:
        frame = self._receive(connection)
        source, target, payload = parse_ipv4(frame, 1)
        if (frame[:6] != mac or source != STATIC or target != destination or
                payload != echo(0, sequence)):
            raise ValueError("wrong next hop, IP endpoint or payload in static echo reply")

    def _exchange(self, connection: socket.socket) -> None:
        self._arp_claim(connection, ZERO, "probe 1")
        self._arp_claim(connection, ZERO, "probe 2")
        # Device RX has armed by the second probe. A request for the tentative
        # address must not displace the expected remaining probe/announcements.
        self._who_has_static(connection)
        self._arp_claim(connection, ZERO, "probe 3")
        self._arp_claim(connection, STATIC, "announcement 1")
        self._arp_claim(connection, STATIC, "announcement 2")
        self._stages.append(STAGES[0])

        self._who_has_static(connection)
        expected = ethernet(HOST_MAC, GUEST_MAC, ARP, arp(2, GUEST_MAC, STATIC, HOST_MAC, LOCAL))
        if self._receive(connection) != expected:
            raise ValueError("static IPv4 direct ARP reply did not match")
        self._send_echo(connection, LOCAL, 1)
        self._expect_echo(connection, LOCAL, HOST_MAC, 1)
        self._stages.append(STAGES[1])

        # A distinct ingress MAC proves this reply goes to the resolved
        # default gateway, while the IP destination stays off-link.
        self._send_echo(connection, REMOTE, 2)
        expected = ethernet(BROADCAST_MAC, GUEST_MAC, ARP,
                            arp(1, GUEST_MAC, STATIC, bytes(6), GATEWAY))
        if self._receive(connection) != expected:
            raise ValueError("static IPv4 did not resolve the configured default gateway")
        self._send(connection, ethernet(GUEST_MAC, GATEWAY_MAC, ARP,
                                         arp(2, GATEWAY_MAC, GATEWAY, GUEST_MAC, STATIC)))
        # The logical peer emits ARP while resolving a gateway. Repeat the
        # request to get the ICMP reply once its next hop is known.
        self._send_echo(connection, REMOTE, 2)
        self._expect_echo(connection, REMOTE, GATEWAY_MAC, 2)
        self._stages.append(STAGES[2])

        self._send(connection, ethernet(BROADCAST_MAC, CONFLICT_MAC, ARP,
                                         arp(1, CONFLICT_MAC, STATIC, bytes(6), STATIC)))
        self._who_has_static(connection)
        self._send_echo(connection, LOCAL, 3)
        if select([connection], [], [], QUIET_SECONDS)[0]:
            raise ValueError("static IPv4 emitted traffic or disconnected after conflict")
        self._stages.append(STAGES[3])

    def _finish_exchange(self) -> dict:
        if tuple(self._stages) != STAGES or (self._packets, self._responses) != (9, 9):
            self.errors.append(f"incomplete static IPv4 exchange: {self._stages!r}")
        return {
            "protocols": ["ARP", "ICMPv4"],
            "static_stages": self._stages,
            "address": "192.0.2.12",
            "gateway": "192.0.2.1",
            "off_link_peer": "198.51.100.7",
            "post_conflict_quiet_seconds": QUIET_SECONDS,
        }
