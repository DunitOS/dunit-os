#!/usr/bin/env python3
"""Exercise the static IPv4 verifier against an independent scripted guest."""
from pathlib import Path
import socket
import struct
import tempfile
import unittest

from net_static_transport import (
    ARP, BROADCAST_MAC, CONFLICT_MAC, GATEWAY, GATEWAY_MAC, IPV4, LOCAL,
    QUIET_SECONDS, REMOTE, STAGES, STATIC, ZERO, NetStaticPeer,
    arp, echo, ethernet, ipv4,
)
from net_test_transport import GUEST_MAC, HOST_MAC


def send_frame(conn, frame):
    conn.sendall(struct.pack("!I", len(frame)) + frame)


def read_frame(conn):
    def read_exact(size):
        data = bytearray()
        while len(data) < size:
            chunk = conn.recv(size - len(data))
            if not chunk:
                raise EOFError("peer closed during wire exchange")
            data.extend(chunk)
        return bytes(data)

    length, = struct.unpack("!I", read_exact(4))
    if not 60 <= length <= 1514:
        raise ValueError(f"invalid peer frame length {length}")
    return read_exact(length)


class StaticPeerTests(unittest.TestCase):
    def test_static_probe_route_and_withdrawal(self):
        result = self.exchange()
        self.assertTrue(result["ok"], result)
        self.assertEqual(result["static_stages"], list(STAGES))
        self.assertEqual((result["packets"], result["responses"]), (9, 9))

    def test_bypassing_the_gateway_is_rejected(self):
        result = self.exchange(wrong_gateway_mac=True)
        self.assertFalse(result["ok"])
        self.assertIn("wrong next hop", " ".join(result["errors"]))

    def test_reply_after_withdrawal_is_rejected(self):
        result = self.exchange(reply_after_conflict=True)
        self.assertFalse(result["ok"])
        self.assertIn("emitted traffic", " ".join(result["errors"]))

    def exchange(self, **case):
        with tempfile.TemporaryDirectory() as directory:
            peer = NetStaticPeer(Path(directory))
            peer.start()
            host, port = peer.endpoint.split(":")
            try:
                with socket.create_connection((host, int(port)), timeout=5) as conn:
                    self.run_guest(conn, **case)
                    done = peer.wait_complete(QUIET_SECONDS + 2)
                    self.assertEqual(done, not any(case.values()))
            finally:
                result = peer.stop()
            return result

    def run_guest(self, conn, *, wrong_gateway_mac=False, reply_after_conflict=False):
        probe = ethernet(BROADCAST_MAC, GUEST_MAC, ARP,
                         arp(1, GUEST_MAC, ZERO, bytes(6), STATIC))
        send_frame(conn, probe)
        send_frame(conn, probe)
        tentative = read_frame(conn)
        self.assertEqual(tentative[:14], BROADCAST_MAC + HOST_MAC + ARP)
        self.assertEqual(tentative[38:42], STATIC)
        send_frame(conn, probe)
        announce = ethernet(BROADCAST_MAC, GUEST_MAC, ARP,
                            arp(1, GUEST_MAC, STATIC, bytes(6), STATIC))
        send_frame(conn, announce)
        send_frame(conn, announce)

        self.assertEqual(read_frame(conn)[38:42], STATIC)
        local_arp = ethernet(HOST_MAC, GUEST_MAC, ARP,
                             arp(2, GUEST_MAC, STATIC, HOST_MAC, LOCAL))
        send_frame(conn, local_arp)
        direct_request = read_frame(conn)
        self.assertEqual(direct_request[26:34], LOCAL + STATIC)
        send_frame(conn, ethernet(HOST_MAC, GUEST_MAC, IPV4,
                                  ipv4(1, STATIC, LOCAL, echo(0, 1))))

        self.assertEqual(read_frame(conn)[26:34], REMOTE + STATIC)
        send_frame(conn, ethernet(BROADCAST_MAC, GUEST_MAC, ARP,
                                  arp(1, GUEST_MAC, STATIC, bytes(6), GATEWAY)))
        gateway_arp = read_frame(conn)
        self.assertEqual(gateway_arp[:14], GUEST_MAC + GATEWAY_MAC + ARP)
        self.assertEqual(gateway_arp[28:32], GATEWAY)
        self.assertEqual(read_frame(conn)[26:34], REMOTE + STATIC)
        next_hop = HOST_MAC if wrong_gateway_mac else GATEWAY_MAC
        send_frame(conn, ethernet(next_hop, GUEST_MAC, IPV4,
                                  ipv4(1, STATIC, REMOTE, echo(0, 2))))
        if wrong_gateway_mac:
            return

        conflict = read_frame(conn)
        self.assertEqual(conflict[:14], BROADCAST_MAC + CONFLICT_MAC + ARP)
        self.assertEqual(conflict[28:32], STATIC)
        self.assertEqual(read_frame(conn)[38:42], STATIC)
        self.assertEqual(read_frame(conn)[26:34], LOCAL + STATIC)
        if reply_after_conflict:
            send_frame(conn, local_arp)


if __name__ == "__main__":
    unittest.main()
