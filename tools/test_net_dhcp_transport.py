#!/usr/bin/env python3
"""Exercise the isolated DHCP verifier against a scripted guest wire exchange."""
import socket
import struct
import tempfile
import unittest
from pathlib import Path

from net_dhcp_transport import (
    ACK, ARP, ARP_HEADER, BROADCAST, BROADCAST_MAC, COOKIE, DISCOVER, FIRST,
    IPV4, NAK, OFFER, REQUEST, SECOND, SERVER, STAGES, ZERO, NetDhcpPeer,
    ipv4, option, parse_request, pseudo,
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
    body += option(53, bytes([kind])) + selectors + b"\xff"
    size = 8 + len(body)
    packet = struct.pack("!HHHH", 68, 67, size, 0) + body
    check = checksum(pseudo(address, target, size) + packet) or 0xffff
    packet = packet[:6] + struct.pack("!H", check) + packet[8:]
    return guest_frame(BROADCAST_MAC if target == BROADCAST else HOST_MAC,
                       IPV4, ipv4(17, address, target, packet))


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

    def test_two_addresses_with_renew_rebind_nak_and_genuine_wire_probes(self):
        with tempfile.TemporaryDirectory() as directory:
            peer = NetDhcpPeer(Path(directory))
            peer.start()
            host, port = peer.endpoint.split(":")
            try:
                with socket.create_connection((host, int(port)), timeout=5) as conn:
                    self.run_script(conn)
                    self.assertTrue(peer.wait_complete(2))
            finally:
                result = peer.stop()
            self.assertTrue(result["ok"], result)
            self.assertEqual(result["lease_stages"], list(STAGES))
            self.assertEqual(result["packets"], 16)
            self.assertEqual(result["responses"], 15)
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

    def read_reply(self, conn, kind, xid, address):
        frame = self.read_frame(conn)
        self.assertEqual(frame[6:14], HOST_MAC + IPV4)
        self.assertEqual(checksum(frame[14:34]), 0)
        self.assertEqual(frame[26:30], SERVER)
        sport, dport, size, check = struct.unpack_from("!HHHH", frame, 34)
        self.assertEqual((sport, dport), (67, 68))
        self.assertNotEqual(check, 0)
        self.assertEqual(checksum(pseudo(SERVER, frame[30:34], size) + frame[34:34 + size]), 0)
        body = frame[42:34 + size]
        self.assertEqual(body[:4], b"\x02\x01\x06\x00")
        self.assertEqual(struct.unpack_from("!I", body, 4)[0], xid)
        self.assertEqual(body[16:20], address)
        self.assertEqual(body[28:34], GUEST_MAC)
        self.assertEqual(body[236:243], COOKIE + option(53, bytes([kind])))

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
        self.assertEqual(echo[12:14], IPV4)
        self.assertEqual(echo[30:34], address)
        total, = struct.unpack_from("!H", echo, 16)
        data = b"\0\0\0\0" + echo[38:14 + total]
        data = data[:2] + struct.pack("!H", checksum(data)) + data[4:]
        self.send_frame(conn, guest_frame(HOST_MAC, IPV4, ipv4(1, address, SERVER, data)))

    def run_script(self, conn):
        self.send_frame(conn, client(DISCOVER, 12))
        self.send_frame(conn, client(DISCOVER, 12))
        self.read_reply(conn, OFFER, 13, bytes([192, 0, 2, 99]))
        self.read_reply(conn, OFFER, 12, FIRST)
        self.send_frame(conn, client(REQUEST, 12, selectors=option(50, FIRST) + option(54, SERVER)))
        self.read_reply(conn, ACK, 12, FIRST)
        self.answer_arp_and_echo(conn, FIRST)

        self.send_frame(conn, guest_frame(BROADCAST_MAC, ARP, ARP_HEADER + b"\0\x01" +
                                          GUEST_MAC + FIRST + bytes(6) + SERVER))
        self.assertEqual(self.read_frame(conn)[12:22], ARP + ARP_HEADER + b"\0\x02")
        self.send_frame(conn, client(REQUEST, 13, FIRST, SERVER))
        self.read_reply(conn, ACK, 13, FIRST)
        self.answer_arp_and_echo(conn, FIRST)

        self.send_frame(conn, client(REQUEST, 14, FIRST, SERVER))
        self.send_frame(conn, client(REQUEST, 14, FIRST, SERVER))
        self.send_frame(conn, client(REQUEST, 14, FIRST, BROADCAST))
        self.read_reply(conn, NAK, 14, ZERO)

        self.send_frame(conn, client(DISCOVER, 15))
        self.read_reply(conn, OFFER, 15, SECOND)
        self.send_frame(conn, client(REQUEST, 15, selectors=option(50, SECOND) + option(54, SERVER)))
        self.read_reply(conn, ACK, 15, SECOND)
        self.answer_arp_and_echo(conn, SECOND, silent=FIRST)


if __name__ == "__main__":
    unittest.main()
