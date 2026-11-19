#!/usr/bin/env python3
"""Exercise the isolated DHCP verifier against a scripted guest wire exchange."""
import socket
import struct
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path

from net_dhcp_transport import (
    ACK, ARP, ARP_HEADER, BROADCAST, BROADCAST_MAC, CLASSLESS_ROUTE, CONFLICT, COOKIE,
    DECLINE, DISCOVER, FIRST, IPV4, NAK, OFFER, PROBER_MAC, REQUEST, SECOND,
    OFF_PREFIX_HOST, ON_LINK_HOST, ON_LINK_MAC, ON_LINK_ROUTE, REMOTE_SERVER, ROUTED_HOST,
    ROUTE_GATEWAY, ROUTE_MAC, SERVER, STAGES, ZERO,
    NetDhcpPeer, Request, client_next_hop, ipv4, option, options, parse_request, pseudo,
)
from net_test_transport import GUEST_MAC, HOST_MAC, checksum


def guest_frame(destination, kind, payload):
    return (destination + GUEST_MAC + kind + payload).ljust(60, b"\0")


def client(kind, xid, address=ZERO, target=BROADCAST, selectors=b""):
    body = bytearray(240)
    body[:4] = b"\x01\x01\x06\x00"
    struct.pack_into("!I", body, 4, xid)
    body[12:16] = address
    body[28:34] = GUEST_MAC
    body[236:240] = COOKIE
    if kind == DISCOVER:
        selectors += option(55, bytes([1, 121, 3, 6]))
    body += option(53, bytes([kind])) + selectors + b"\xff"
    size = 8 + len(body)
    packet = struct.pack("!HHHH", 68, 67, size, 0) + body
    check = checksum(pseudo(address, target, size) + packet) or 0xffff
    packet = packet[:6] + struct.pack("!H", check) + packet[8:]
    return guest_frame(client_next_hop(target), IPV4, ipv4(17, address, target, packet))


class DhcpPeerTests(unittest.TestCase):
    def test_bounded_request_validation(self):
        frame = client(DISCOVER, 12)
        self.assertEqual(parse_request(frame, DISCOVER, ZERO, BROADCAST).xid, 12)
        malformed = bytearray(frame)
        malformed[-1] ^= 1
        cases = (
            bytes(malformed),
            frame[:80],
            client(DISCOVER, 12, selectors=option(53, bytes([DISCOVER]))),
            client(DISCOVER, 12, selectors=option(255, b"nonzero padding")),
            client(REQUEST, 12),
            client(DISCOVER, 12, target=SERVER),
        )
        for index, case in enumerate(cases):
            with self.subTest(index=index), self.assertRaises(ValueError):
                parse_request(case, DISCOVER, ZERO, BROADCAST)

    def test_discovery_earlier_than_conflict_cooldown_is_rejected(self):
        peer = NetDhcpPeer(Path("."))
        before = Request(12, BROADCAST, {})
        after = Request(13, BROADCAST, {})
        decline = client(DECLINE, 12, selectors=option(50, CONFLICT) + option(54, SERVER))
        with (patch.object(peer, "_expect_acd"), patch.object(peer, "_send"),
              patch.object(peer, "_receive", return_value=decline),
              patch.object(peer, "_discover", return_value=after),
              patch("net_dhcp_transport.monotonic", side_effect=(20.0, 20.1))):
            with self.assertRaisesRegex(ValueError, "bypassed DECLINE cooldown"):
                peer._decline(None, before)

    def test_conflict_then_two_addresses_with_renewal_and_rebind(self):
        with tempfile.TemporaryDirectory() as directory:
            peer = NetDhcpPeer(Path(directory))
            peer.start()
            host, port = peer.endpoint.split(":")
            try:
                with socket.create_connection((host, int(port)), timeout=5) as conn:
                    with patch("net_dhcp_transport.monotonic", side_effect=(20.0, 30.125)):
                        self.run_script(conn)
                        self.assertTrue(peer.wait_complete(2))
            finally:
                result = peer.stop()
            self.assertTrue(result["ok"], result)
            self.assertEqual(result["lease_stages"], list(STAGES))
            self.assertEqual(result["packets"], 36)
            self.assertEqual(result["responses"], 31)
            self.assertEqual(result["acd_probes"], 7)
            self.assertEqual(result["acd_announcements"], 4)
            self.assertEqual(result["decline_wall_cooldown_seconds"], 10.125)
            self.assertEqual(result["renewal_retries_before_rebind"], 1)

    def read_frame(self, conn):
        def read_exact(size):
            data = b""
            while len(data) < size:
                new = conn.recv(size - len(data))
                self.assertTrue(new)
                data += new
            return data

        size, = struct.unpack("!I", read_exact(4))
        self.assertTrue(60 <= size <= 1514)
        return read_exact(size)

    @staticmethod
    def send_frame(conn, frame):
        conn.sendall(struct.pack("!I", len(frame)) + frame)

    def read_reply(self, conn, kind, xid, address, *, includes_routes=True,
                   server=SERVER, relay=ZERO):
        frame = self.read_frame(conn)
        self.assertEqual(frame[6:14], client_next_hop(server) + IPV4)
        self.assertEqual(checksum(frame[14:34]), 0)
        source = relay if relay != ZERO else server
        self.assertEqual(frame[26:30], source)
        sport, dport, size, check = struct.unpack_from("!HHHH", frame, 34)
        self.assertEqual((sport, dport), (67, 68))
        self.assertNotEqual(check, 0)
        self.assertEqual(checksum(pseudo(source, frame[30:34], size) + frame[34:34 + size]), 0)
        body = frame[42:34 + size]
        self.assertEqual(body[:4], bytes([2, 1, 6, int(relay != ZERO)]))
        self.assertEqual(struct.unpack_from("!I", body, 4)[0], xid)
        self.assertEqual(body[16:20], address)
        self.assertEqual(body[24:28], relay)
        self.assertEqual(body[28:34], GUEST_MAC)
        self.assertEqual(body[236:243], COOKIE + option(53, bytes([kind])))
        parsed = options(body[240:])
        self.assertEqual(parsed.get(54), server)
        routes = CLASSLESS_ROUTE if address == SECOND else CLASSLESS_ROUTE + ON_LINK_ROUTE
        self.assertEqual(parsed.get(121), routes if includes_routes and kind != NAK else None)
        self.assertEqual(parsed.get(3), SERVER if includes_routes and kind != NAK else None)

    def answer_arp_and_echo(self, conn, address, silent=None):
        if silent is not None:
            stale = self.read_frame(conn)
            self.assertEqual(stale[38:42], silent)
        arp = self.read_frame(conn)
        self.assertEqual(arp[12:22], ARP + ARP_HEADER + b"\0\x01")
        self.assertEqual(arp[38:42], address)
        self.send_frame(conn, guest_frame(HOST_MAC, ARP, ARP_HEADER + b"\0\x02" +
                                         GUEST_MAC + address + HOST_MAC + SERVER))
        echo = self.read_frame(conn)
        self.answer_echo(conn, echo, address, SERVER, HOST_MAC)

    def answer_echo(self, conn, echo, address, remote, next_hop_mac):
        self.assertEqual(echo[12:14], IPV4)
        self.assertEqual(echo[26:34], remote + address)
        total, = struct.unpack_from("!H", echo, 16)
        data = b"\0\0\0\0" + echo[38:14 + total]
        data = data[:2] + struct.pack("!H", checksum(data)) + data[4:]
        self.send_frame(conn, guest_frame(next_hop_mac, IPV4, ipv4(1, address, remote, data)))

    def answer_classless(self, conn, address, *, resolve):
        off_prefix = self.read_frame(conn)
        self.assertEqual(off_prefix[26:34], OFF_PREFIX_HOST + address)
        routed = self.read_frame(conn)
        self.assertEqual(routed[26:34], ROUTED_HOST + address)
        if resolve:
            self.send_frame(conn, guest_frame(BROADCAST_MAC, ARP, ARP_HEADER + b"\0\x01" +
                                             GUEST_MAC + address + bytes(6) + ROUTE_GATEWAY))
            reply = self.read_frame(conn)
            self.assertEqual(reply[:6], GUEST_MAC)
            self.assertEqual(reply[6:12], ROUTE_MAC)
            self.assertEqual(reply[22:32], ROUTE_MAC + ROUTE_GATEWAY)
            self.assertEqual(reply[32:42], GUEST_MAC + address)
            self.assertEqual(self.read_frame(conn), routed)
        self.answer_echo(conn, routed, address, ROUTED_HOST, ROUTE_MAC)

    def answer_on_link(self, conn, address):
        self.answer_echo(conn, self.read_frame(conn), address, ON_LINK_HOST, ON_LINK_MAC)

    def acd_request(self, conn, candidate, sender=ZERO):
        self.send_frame(conn, guest_frame(BROADCAST_MAC, ARP, ARP_HEADER + b"\0\x01" +
                                          GUEST_MAC + sender + bytes(6) + candidate))

    def complete_acd(self, conn, address):
        self.acd_request(conn, address)
        tentative = self.read_frame(conn)
        self.assertEqual(tentative[12:22], ARP + ARP_HEADER + b"\0\x01")
        self.assertEqual(tentative[38:42], address)
        for _ in range(2):
            self.acd_request(conn, address)
        for _ in range(2):
            self.acd_request(conn, address, address)

    def answer_active_probe(self, conn, address):
        probe = self.read_frame(conn)
        self.assertEqual(probe[6:12], PROBER_MAC)
        self.assertEqual(probe[12:22], ARP + ARP_HEADER + b"\0\x01")
        self.assertEqual(probe[28:32], ZERO)
        self.assertEqual(probe[38:42], address)
        self.send_frame(conn, guest_frame(PROBER_MAC, ARP, ARP_HEADER + b"\0\x02" +
                                         GUEST_MAC + address + PROBER_MAC + ZERO))

    def run_script(self, conn):
        self.send_frame(conn, client(DISCOVER, 12))
        self.send_frame(conn, client(DISCOVER, 12))
        self.read_reply(conn, OFFER, 13, FIRST)
        self.read_reply(conn, OFFER, 12, CONFLICT)
        self.send_frame(conn, client(REQUEST, 12, selectors=option(50, CONFLICT) + option(54, SERVER)))
        self.read_reply(conn, ACK, 12, CONFLICT)
        self.acd_request(conn, CONFLICT)
        conflict = self.read_frame(conn)
        self.assertEqual(conflict[28:32], CONFLICT)
        self.assertEqual(conflict[38:42], CONFLICT)
        self.send_frame(conn, client(DECLINE, 12, selectors=option(50, CONFLICT) + option(54, SERVER)))
        self.assertEqual(self.read_frame(conn)[38:42], CONFLICT)

        self.send_frame(conn, client(DISCOVER, 13))
        self.read_reply(conn, OFFER, 13, FIRST, server=REMOTE_SERVER, relay=ROUTE_GATEWAY)
        self.send_frame(conn, client(REQUEST, 13, selectors=option(50, FIRST) + option(54, REMOTE_SERVER)))
        self.read_reply(conn, ACK, 13, FIRST, server=REMOTE_SERVER, relay=ROUTE_GATEWAY)
        self.complete_acd(conn, FIRST)
        self.answer_active_probe(conn, FIRST)
        self.answer_arp_and_echo(conn, FIRST)
        self.answer_classless(conn, FIRST, resolve=True)
        self.answer_on_link(conn, FIRST)

        self.send_frame(conn, guest_frame(BROADCAST_MAC, ARP, ARP_HEADER + b"\0\x01" +
                                          GUEST_MAC + FIRST + bytes(6) + ROUTE_GATEWAY))
        self.assertEqual(self.read_frame(conn)[12:22], ARP + ARP_HEADER + b"\0\x02")
        self.send_frame(conn, client(REQUEST, 13, FIRST, REMOTE_SERVER))
        self.read_reply(conn, ACK, 13, FIRST, includes_routes=False, server=REMOTE_SERVER)
        self.answer_arp_and_echo(conn, FIRST)
        self.answer_classless(conn, FIRST, resolve=False)
        self.answer_on_link(conn, FIRST)

        self.send_frame(conn, client(REQUEST, 14, FIRST, REMOTE_SERVER))
        self.send_frame(conn, client(REQUEST, 14, FIRST, REMOTE_SERVER))
        self.send_frame(conn, client(REQUEST, 14, FIRST, BROADCAST))
        self.read_reply(conn, NAK, 14, ZERO, server=REMOTE_SERVER, relay=ROUTE_GATEWAY)

        self.send_frame(conn, client(DISCOVER, 15))
        self.read_reply(conn, OFFER, 15, SECOND)
        self.send_frame(conn, client(REQUEST, 15, selectors=option(50, SECOND) + option(54, SERVER)))
        self.read_reply(conn, ACK, 15, SECOND)
        self.complete_acd(conn, SECOND)
        retired_route = self.read_frame(conn)
        self.assertEqual(retired_route[:6], GUEST_MAC)
        self.assertEqual(retired_route[6:12], ON_LINK_MAC)
        self.assertEqual(retired_route[26:34], ON_LINK_HOST + SECOND)
        self.answer_arp_and_echo(conn, SECOND, silent=FIRST)


if __name__ == "__main__":
    unittest.main()
