"""Isolated DHCPv4 server and Ethernet verifier for the production netd."""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import socket
import struct
from time import monotonic

from net_test_transport import GUEST_MAC, HOST_MAC, EthernetPeer, checksum

ZERO = bytes(4)
BROADCAST = bytes([255]) * 4
BROADCAST_MAC = bytes([255]) * 6
SERVER = bytes([192, 0, 2, 1])
FIRST = bytes([192, 0, 2, 2])
SECOND = bytes([192, 0, 2, 3])
CONFLICT = bytes([192, 0, 2, 99])
PROBER_MAC = bytes.fromhex("020000000004")
ROUTE_GATEWAY = bytes([192, 0, 2, 254])
ROUTE_MAC = bytes.fromhex("020000000007")
ROUTED_HOST = bytes([198, 51, 100, 7])
REMOTE_SERVER = bytes([198, 51, 100, 53])
ON_LINK_HOST = bytes([198, 51, 101, 7])
ON_LINK_MAC = bytes.fromhex("020000000008")
OFF_PREFIX_HOST = bytes([203, 0, 113, 7])
CLASSLESS_ROUTE = bytes([24]) + ROUTED_HOST[:3] + ROUTE_GATEWAY
ON_LINK_ROUTE = bytes([24]) + ON_LINK_HOST[:3] + ZERO
MASK = bytes([255, 255, 255, 0])
DNS = bytes([192, 0, 2, 53])
IPV4 = b"\x08\x00"
ARP = b"\x08\x06"
ARP_HEADER = struct.pack("!HHBB", 1, 0x0800, 6, 4)
COOKIE = b"\x63\x82\x53\x63"
DISCOVER, OFFER, REQUEST, DECLINE, ACK, NAK = 1, 2, 3, 4, 5, 6
# The guest verifies ten seconds of monotonic cooldown. Allow up to one second
# of wall-clock skew when measuring the same interval across the socket.
MIN_DECLINE_WALL_SECONDS = 9.0
STAGES = (
    "discover_retried_after_loss",
    "select_ignored_wrong_transaction",
    "conflict_probe_declined_and_cooled_down",
    "relayed_offer_and_ack",
    "first_lease_acd_and_probe_response",
    "first_lease_arp_icmp",
    "classless_route_and_router_option_ignored",
    "on_link_route_direct_reply",
    "unicast_renewal_to_remote_server_ack",
    "renewed_lease_arp_icmp",
    "renewal_retains_classless_route",
    "renewal_retains_on_link_route",
    "unicast_renewal_lost_rebind_relayed_nak",
    "new_lease_acd",
    "new_lease_arp_icmp_old_address_silent",
    "replacement_lease_removes_on_link_route",
)


def ethernet(destination: bytes, kind: bytes, payload: bytes, *, source: bytes = HOST_MAC) -> bytes:
    return (destination + source + kind + payload).ljust(60, b"\0")


def ipv4(protocol: int, src: bytes, dst: bytes, payload: bytes) -> bytes:
    header = struct.pack("!BBHHHBBH4s4s", 0x45, 0, 20 + len(payload), 0,
                         0x4000, 64, protocol, 0, src, dst)
    return header[:10] + struct.pack("!H", checksum(header)) + header[12:] + payload


def parse_ipv4(frame: bytes, protocol: int) -> tuple[bytes, bytes, bytes]:
    header = frame[14:34]
    if frame[6:14] != GUEST_MAC + IPV4 or len(header) != 20:
        raise ValueError("expected guest IPv4 Ethernet frame")
    total = int.from_bytes(header[2:4], "big")
    fragment = int.from_bytes(header[6:8], "big")
    if (header[0] != 0x45 or not 20 <= total <= len(frame) - 14 or
            fragment & ~0x4000 or not header[8] or header[9] != protocol or checksum(header)):
        raise ValueError("invalid guest IPv4 header")
    return header[12:16], header[16:20], frame[34:14 + total]


def pseudo(src: bytes, dst: bytes, length: int) -> bytes:
    return struct.pack("!4s4sBBH", src, dst, 0, 17, length)


def option(kind: int, value: bytes) -> bytes:
    return bytes([kind, len(value)]) + value


def options(data: bytes) -> dict[int, bytes]:
    result: dict[int, bytes] = {}
    while data:
        kind = data[0]
        data = data[1:]
        if kind == 0:
            continue
        if kind == 255:
            if any(data):
                raise ValueError("nonzero DHCP padding")
            return result
        if not data or data[0] > len(data) - 1 or kind in result:
            raise ValueError("invalid guest DHCP options")
        length = data[0]
        result[kind], data = data[1:1 + length], data[1 + length:]
    raise ValueError("DHCP end option missing")


@dataclass(frozen=True)
class Request:
    xid: int
    destination: bytes
    options: dict[int, bytes]


def client_next_hop(destination: bytes) -> bytes:
    if destination == BROADCAST:
        return BROADCAST_MAC
    return ROUTE_MAC if destination == REMOTE_SERVER else HOST_MAC


def parse_request(frame: bytes, kind: int, address: bytes, destination: bytes) -> Request:
    source, target, data = parse_ipv4(frame, 17)
    if target != destination or frame[:6] != client_next_hop(target):
        raise ValueError("invalid DHCP Ethernet or IP destination")
    if source != address or len(data) < 248:
        raise ValueError("invalid DHCP source address or length")
    sport, dport, length, check = struct.unpack_from("!HHHH", data)
    if (sport, dport, length) != (68, 67, len(data)) or not check or checksum(pseudo(source, target, length) + data):
        raise ValueError("invalid DHCP UDP header or checksum")
    body = data[8:]
    if (body[:4] != b"\x01\x01\x06\x00" or body[12:16] != address or
            any(body[16:28]) or body[28:34] != GUEST_MAC or body[236:240] != COOKIE):
        raise ValueError("invalid DHCP BOOTP envelope")
    parsed = options(body[240:])
    if parsed.get(53) != bytes([kind]):
        raise ValueError(f"expected DHCP message type {kind}, got {parsed.get(53)!r}")
    return Request(struct.unpack_from("!I", body, 4)[0], target, parsed)


def reply(kind: int, xid: int, address: bytes, *, unicast: bool = False,
          include_routes: bool = True, server: bytes = SERVER, relay: bytes = ZERO) -> bytes:
    body = bytearray(240)
    body[:4] = bytes([2, 1, 6, int(relay != ZERO)])
    struct.pack_into("!I", body, 4, xid)
    body[16:20] = address
    body[24:28] = relay
    body[28:34] = GUEST_MAC
    body[236:240] = COOKIE
    body += option(53, bytes([kind])) + option(54, server)
    if kind != NAK:
        body += option(1, MASK) + option(6, DNS)
        if include_routes:
            routes = CLASSLESS_ROUTE if address == SECOND else CLASSLESS_ROUTE + ON_LINK_ROUTE
            body += option(121, routes) + option(3, SERVER)
        body += option(51, struct.pack("!I", 28))
        body += option(58, struct.pack("!I", 14))
        body += option(59, struct.pack("!I", 20))
    body += b"\xff"
    body = body.ljust(300, b"\0")
    target = address if unicast else BROADCAST
    source = relay if relay != ZERO else server
    length = 8 + len(body)
    datagram = struct.pack("!HHHH", 67, 68, length, 0) + body
    check = checksum(pseudo(source, target, length) + datagram) or 0xffff
    payload = datagram[:6] + struct.pack("!H", check) + datagram[8:]
    return ethernet(GUEST_MAC if unicast else BROADCAST_MAC,
                    IPV4, ipv4(17, source, target, payload),
                    source=client_next_hop(server))


class NetDhcpPeer(EthernetPeer):
    """Verify link traffic through address acquisition, lease loss and replacement."""

    def __init__(self, workdir: Path):
        super().__init__(workdir)
        self._stages: list[str] = []
        self._renew_retries = 0
        self._acd_probes = 0
        self._acd_announcements = 0
        self._cooldown = None

    def wait_complete(self, timeout: float) -> bool:
        self._thread.join(timeout=timeout)
        return tuple(self._stages) == STAGES and not self._thread.is_alive()

    def _discover(self, connection: socket.socket) -> Request:
        request = parse_request(self._receive(connection), DISCOVER, ZERO, BROADCAST)
        if 50 in request.options or 54 in request.options:
            raise ValueError("fresh DHCP discovery retained a former lease")
        requested = request.options.get(55, b"")
        if 121 not in requested or 3 not in requested or requested.index(121) > requested.index(3):
            raise ValueError("DHCP discovery must request classless routes before the router option")
        return request

    def _select(self, connection: socket.socket, discover: Request, address: bytes,
                *, server: bytes = SERVER, relay: bytes = ZERO) -> None:
        self._send(connection, reply(OFFER, discover.xid, address, server=server, relay=relay))
        request = parse_request(self._receive(connection), REQUEST, ZERO, BROADCAST)
        if (request.xid != discover.xid or request.options.get(50) != address or
                request.options.get(54) != server):
            raise ValueError("client did not select the matching DHCP offer")
        self._send(connection, reply(ACK, request.xid, address, server=server, relay=relay))

    def _expect_acd(self, connection: socket.socket, address: bytes, sender: bytes) -> None:
        expected = (BROADCAST_MAC + GUEST_MAC + ARP + ARP_HEADER + b"\0\x01" +
                    GUEST_MAC + sender + bytes(6) + address).ljust(60, b"\0")
        if self._receive(connection) != expected:
            raise ValueError(f"invalid RFC 5227 ARP frame for {list(address)}")
        if sender == ZERO:
            self._acd_probes += 1
        else:
            self._acd_announcements += 1

    def _complete_acd(self, connection: socket.socket, address: bytes) -> None:
        self._expect_acd(connection, address, ZERO)
        # A tentative address must stay silent while we solicit it.
        self._send(connection, self._arp_request(address))
        for _ in range(2):
            self._expect_acd(connection, address, ZERO)
        for _ in range(2):
            self._expect_acd(connection, address, address)

    def _active_probe_reply(self, connection: socket.socket, address: bytes) -> None:
        probe = (BROADCAST_MAC + PROBER_MAC + ARP + ARP_HEADER + b"\0\x01" +
                 PROBER_MAC + ZERO + bytes(6) + address).ljust(60, b"\0")
        self._send(connection, probe)
        expected = (PROBER_MAC + GUEST_MAC + ARP + ARP_HEADER + b"\0\x02" +
                    GUEST_MAC + address + PROBER_MAC + ZERO).ljust(60, b"\0")
        if self._receive(connection) != expected:
            raise ValueError("active address did not answer an incoming zero-SPA ARP probe")

    def _decline(self, connection: socket.socket, before: Request) -> Request:
        self._expect_acd(connection, CONFLICT, ZERO)
        self._send(connection, ethernet(BROADCAST_MAC, ARP, ARP_HEADER + b"\0\x01" +
                                        HOST_MAC + CONFLICT + bytes(6) + CONFLICT))
        decline = parse_request(self._receive(connection), DECLINE, ZERO, BROADCAST)
        if decline.options.get(50) != CONFLICT or decline.options.get(54) != SERVER:
            raise ValueError("DHCP DECLINE did not identify the conflicting lease")
        began = monotonic()
        self._send(connection, self._arp_request(CONFLICT))
        discover = self._discover(connection)
        self._cooldown = monotonic() - began
        if self._cooldown < MIN_DECLINE_WALL_SECONDS:
            raise ValueError(f"DHCP DISCOVER bypassed DECLINE cooldown ({self._cooldown:.3f}s)")
        if discover.xid == before.xid:
            raise ValueError("DHCP DISCOVER retained the declined transaction ID")
        return discover

    def _probe_address(self, connection: socket.socket, address: bytes,
                       sequence: int, *, silent: bytes | None = None) -> None:
        if silent is not None:
            self._send(connection, self._arp_request(silent))
        self._send(connection, self._arp_request(address))
        response = (GUEST_MAC + address + HOST_MAC + SERVER)
        arp = self._receive(connection)
        expected = (HOST_MAC + GUEST_MAC + ARP + ARP_HEADER + b"\0\x02" + response).ljust(60, b"\0")
        if arp != expected:
            raise ValueError(f"wrong ARP response for leased IPv4 address {list(address)}")
        echo = self._echo_request(sequence)
        self._send(connection, ethernet(GUEST_MAC, IPV4, ipv4(1, SERVER, address, echo)))
        self._expect_echo_reply(self._receive(connection), address, SERVER, HOST_MAC, echo)

    @staticmethod
    def _echo_request(sequence: int) -> bytes:
        echo = struct.pack("!BBHHH", 8, 0, 0, 0xd017, sequence) + b"dunit-dhcp-lease"
        return echo[:2] + struct.pack("!H", checksum(echo)) + echo[4:]

    @staticmethod
    def _expect_echo_reply(frame: bytes, address: bytes, remote: bytes,
                           next_hop_mac: bytes, echo: bytes) -> None:
        source, target, payload = parse_ipv4(frame, 1)
        if (source != address or target != remote or frame[:6] != next_hop_mac or
                payload[:2] != b"\0\0" or payload[4:] != echo[4:] or checksum(payload)):
            raise ValueError(f"wrong ICMP echo reply for leased IPv4 address {list(address)}")

    def _probe_classless_route(self, connection: socket.socket, address: bytes,
                               sequence: int, *, expect_arp: bool = False) -> None:
        # Option 121 contains no default route, so the off-prefix packet
        # must produce neither an echo reply nor ARP for the option-3 router.
        echo = self._echo_request(sequence)
        self._send(connection, ethernet(GUEST_MAC, IPV4, ipv4(1, OFF_PREFIX_HOST, address, echo)))
        routed = ethernet(GUEST_MAC, IPV4, ipv4(1, ROUTED_HOST, address, echo))
        self._send(connection, routed)
        frame = self._receive(connection)
        if expect_arp or frame[12:14] == ARP:
            expected = (BROADCAST_MAC + GUEST_MAC + ARP + ARP_HEADER + b"\0\x01" +
                        GUEST_MAC + address + bytes(6) + ROUTE_GATEWAY).ljust(60, b"\0")
            if frame != expected:
                raise ValueError("expected ARP for the classless route gateway")
            reply = ARP_HEADER + b"\0\x02" + ROUTE_MAC + ROUTE_GATEWAY + GUEST_MAC + address
            self._send(connection, ethernet(GUEST_MAC, ARP, reply, source=ROUTE_MAC))
            self._send(connection, routed)
            frame = self._receive(connection)
        self._expect_echo_reply(frame, address, ROUTED_HOST, ROUTE_MAC, echo)

    @staticmethod
    def _arp_request(address: bytes) -> bytes:
        return ethernet(BROADCAST_MAC, ARP, ARP_HEADER + b"\0\x01" +
                        HOST_MAC + SERVER + bytes(6) + address)

    def _send_on_link_echo(self, connection: socket.socket, address: bytes, sequence: int) -> bytes:
        echo = self._echo_request(sequence)
        self._send(connection, ethernet(GUEST_MAC, IPV4, ipv4(1, ON_LINK_HOST, address, echo),
                                        source=ON_LINK_MAC))
        return echo

    def _probe_on_link_route(self, connection: socket.socket, address: bytes, sequence: int) -> None:
        echo = self._send_on_link_echo(connection, address, sequence)
        self._expect_echo_reply(self._receive(connection), address, ON_LINK_HOST, ON_LINK_MAC, echo)

    def _renewal(self, connection: socket.socket, *, allow_rebind: bool = False) -> Request:
        # The stack may need one gateway/server ARP resolution before sending
        # the unicast renewal. The server is off-link, reachable via the relay.
        for _ in range(8):
            frame = self._receive(connection)
            if frame[12:14] == ARP:
                request = (BROADCAST_MAC + GUEST_MAC + ARP + ARP_HEADER + b"\0\x01" +
                           GUEST_MAC + FIRST + bytes(6) + ROUTE_GATEWAY).ljust(60, b"\0")
                if frame != request:
                    raise ValueError("unexpected ARP while renewing lease")
                self._send(connection, ethernet(GUEST_MAC, ARP, ARP_HEADER + b"\0\x02" +
                                                ROUTE_MAC + ROUTE_GATEWAY + GUEST_MAC + FIRST,
                                                source=ROUTE_MAC))
                continue
            destination = BROADCAST if allow_rebind and frame[:6] == BROADCAST_MAC else REMOTE_SERVER
            packet = parse_request(frame, REQUEST, FIRST, destination)
            if 50 in packet.options or 54 in packet.options:
                raise ValueError("DHCP renewal/rebind used discovery-only selectors")
            return packet
        raise ValueError("ARP retries exhausted while waiting for DHCP renewal")

    def _exchange(self, connection: socket.socket) -> None:
        self._discover(connection)  # Intentionally lose the first discovery.
        # E1000 suppresses receives for 1000 ms after RX setup. The first
        # dropped frame normally covers this interval; avoid relying on it.
        if self._stop.wait(1.25):
            return
        discover = self._discover(connection)
        self._stages.append("discover_retried_after_loss")
        self._send(connection, reply(OFFER, discover.xid ^ 1, FIRST))
        self._select(connection, discover, CONFLICT)
        self._stages.append("select_ignored_wrong_transaction")
        discover = self._decline(connection, discover)
        self._stages.append("conflict_probe_declined_and_cooled_down")

        self._select(connection, discover, FIRST, server=REMOTE_SERVER, relay=ROUTE_GATEWAY)
        self._stages.append("relayed_offer_and_ack")
        self._complete_acd(connection, FIRST)
        self._active_probe_reply(connection, FIRST)
        self._stages.append("first_lease_acd_and_probe_response")
        self._probe_address(connection, FIRST, 1)
        self._stages.append("first_lease_arp_icmp")
        self._probe_classless_route(connection, FIRST, 4, expect_arp=True)
        self._stages.append("classless_route_and_router_option_ignored")
        self._probe_on_link_route(connection, FIRST, 6)
        self._stages.append("on_link_route_direct_reply")

        renewal = self._renewal(connection)
        self._send(connection, reply(ACK, renewal.xid, FIRST, unicast=True, include_routes=False,
                                     server=REMOTE_SERVER))
        self._stages.append("unicast_renewal_to_remote_server_ack")
        self._probe_address(connection, FIRST, 2)
        self._stages.append("renewed_lease_arp_icmp")
        self._probe_classless_route(connection, FIRST, 5)
        self._stages.append("renewal_retains_classless_route")
        self._probe_on_link_route(connection, FIRST, 7)
        self._stages.append("renewal_retains_on_link_route")

        self._renewal(connection)  # Drop this renewal until T2.
        for _ in range(8):
            request = self._renewal(connection, allow_rebind=True)
            if request.destination == BROADCAST:
                self._send(connection, reply(NAK, request.xid, ZERO,
                                             server=REMOTE_SERVER, relay=ROUTE_GATEWAY))
                break
            self._renew_retries += 1
        else:
            raise ValueError("client did not broadcast DHCP rebind")
        self._stages.append("unicast_renewal_lost_rebind_relayed_nak")

        discover = self._discover(connection)
        self._select(connection, discover, SECOND)
        self._complete_acd(connection, SECOND)
        self._stages.append("new_lease_acd")
        # No ARP or echo may precede the expected response from the new lease.
        self._send_on_link_echo(connection, SECOND, 8)
        self._probe_address(connection, SECOND, 3, silent=FIRST)
        self._stages.append("new_lease_arp_icmp_old_address_silent")
        self._stages.append("replacement_lease_removes_on_link_route")

    def _finish_exchange(self) -> dict:
        if tuple(self._stages) != STAGES:
            self.errors.append(f"incomplete DHCP exchange; stages={self._stages!r}")
        return {
            "protocols": ["ARP", "ICMPv4", "DHCPv4"],
            "lease_stages": self._stages,
            "renewal_retries_before_rebind": self._renew_retries,
            "acd_probes": self._acd_probes,
            "acd_announcements": self._acd_announcements,
            "decline_wall_cooldown_seconds": self._cooldown,
        }
