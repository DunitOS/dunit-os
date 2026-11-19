#!/usr/bin/env python3
"""Host checks for the bounded native application socket wire fixture."""
from __future__ import annotations

from pathlib import Path
import socket
import struct
import tempfile
import unittest

from net_dhcp_transport import ARP, BROADCAST_MAC, IPV4, ipv4
from net_live_transport import ACK, FIN, PSH, SYN, advance, pseudo
from net_socket_transport import (
    CONFLICT_MAC, GUEST, HOST, STAGES, TCP_LENGTH, TCP_PORT, TCP_REPLY, UDP_PORT, UDP_REPLY,
    UDP_REQUEST, NetSocketPeer, check_udp, host_tcp, host_udp,
)
from net_static_transport import arp, ethernet
from net_test_transport import GUEST_MAC, HOST_MAC, checksum


def guest_ip(protocol: int, payload: bytes) -> bytes:
    return ethernet(HOST_MAC, GUEST_MAC, IPV4, ipv4(protocol, GUEST, HOST, payload))


def guest_udp() -> bytes:
    length = 8 + len(UDP_REQUEST)
    datagram = struct.pack("!HHHH", 49222, UDP_PORT, length, 0) + UDP_REQUEST
    check = checksum(pseudo(GUEST, HOST, 17, length) + datagram) or 0xffff
    return guest_ip(17, datagram[:6] + struct.pack("!H", check) + datagram[8:])


def guest_tcp(seq: int, ack: int, flags: int, data: bytes = b"") -> bytes:
    header = struct.pack("!HHIIBBHHH", 49223, TCP_PORT, seq, ack, 0x50, flags, 4096, 0, 0)
    segment = header + data
    check = checksum(pseudo(GUEST, HOST, 6, len(segment)) + segment)
    return guest_ip(6, segment[:16] + struct.pack("!H", check) + segment[18:])


def send(connection: socket.socket, frame: bytes) -> None:
    connection.sendall(struct.pack("!I", len(frame)) + frame)


def receive(connection: socket.socket) -> bytes:
    def read(size: int) -> bytes:
        result = bytearray()
        while len(result) < size:
            part = connection.recv(size - len(result))
            if not part:
                raise AssertionError("host fixture closed before completing the frame")
            result.extend(part)
        return bytes(result)
    return read(struct.unpack("!I", read(4))[0])


class SocketPeerTests(unittest.TestCase):
    def test_checked_full_exchange(self):
        with tempfile.TemporaryDirectory() as workdir:
            peer = NetSocketPeer(Path(workdir))
            peer.start()
            try:
                host, port = peer.endpoint.split(":")
                with socket.create_connection((host, int(port)), timeout=3) as connection:
                    for address in [bytes(4)] * 3 + [GUEST] * 2:
                        send(connection, ethernet(BROADCAST_MAC, GUEST_MAC, ARP,
                                                  arp(1, GUEST_MAC, address, bytes(6), GUEST)))
                    send(connection, ethernet(BROADCAST_MAC, GUEST_MAC, ARP,
                                              arp(1, GUEST_MAC, GUEST, bytes(6), HOST)))
                    self.assertEqual(receive(connection), ethernet(
                        GUEST_MAC, HOST_MAC, ARP, arp(2, HOST_MAC, HOST, GUEST_MAC, GUEST)))
                    send(connection, guest_udp())
                    self.assertEqual(receive(connection), host_udp(49222, UDP_REPLY))

                    # Wrap the guest's sequence counter while submitting data.
                    guest_seq = 0xfffffff0
                    host_seq = 0x50900000
                    send(connection, guest_tcp(guest_seq, 0, SYN))
                    guest_seq = advance(guest_seq, 1)
                    self.assertEqual(receive(connection), host_tcp(49223, host_seq, guest_seq,
                                                                     SYN | ACK))
                    host_seq = advance(host_seq, 1)
                    send(connection, guest_tcp(guest_seq, host_seq, ACK))
                    for first in range(0, TCP_LENGTH, 500):
                        data = bytes(index % 251 for index in range(first, first + 500))
                        send(connection, guest_tcp(guest_seq, host_seq, ACK | PSH, data))
                        guest_seq = advance(guest_seq, len(data))
                        self.assertEqual(receive(connection), host_tcp(49223, host_seq, guest_seq,
                                                                         ACK))
                    first, last = TCP_REPLY[:13], TCP_REPLY[13:]
                    self.assertEqual(receive(connection), host_tcp(
                        49223, host_seq, guest_seq, ACK | PSH, first))
                    host_seq = advance(host_seq, len(first))
                    send(connection, guest_tcp(guest_seq, host_seq, ACK))
                    self.assertEqual(receive(connection), host_tcp(
                        49223, host_seq, guest_seq, ACK | PSH | FIN, last))
                    host_seq = advance(host_seq, len(last) + 1)
                    send(connection, guest_tcp(guest_seq, host_seq, ACK | FIN))
                    self.assertEqual(receive(connection), host_tcp(
                        49223, host_seq, advance(guest_seq, 1), ACK))
                    self.assertEqual(receive(connection), ethernet(
                        BROADCAST_MAC, CONFLICT_MAC, ARP,
                        arp(1, CONFLICT_MAC, GUEST, bytes(6), GUEST)))
                    self.assertTrue(peer.wait_complete(3))
            finally:
                result = peer.stop()
            self.assertEqual(result["socket_stages"], list(STAGES))
            self.assertEqual(result["tcp_bytes"], TCP_LENGTH)
            self.assertTrue(result["ok"], result)

    def test_rejects_bad_udp_checksum_and_payload(self):
        frame = bytearray(guest_udp())
        self.assertEqual(check_udp(frame), 49222)
        frame[42] ^= 1
        with self.assertRaisesRegex(ValueError, "payload or checksum"):
            check_udp(frame)


if __name__ == "__main__":
    unittest.main()
