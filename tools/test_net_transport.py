#!/usr/bin/env python3
"""Exercise the actual framed peer, including shutdown and hostile framing."""
import json
from pathlib import Path
import socket
import struct
import tempfile
import time
import unittest

from net_test_transport import FRAME_SIZE, NetPeer, PACKETS, request_frame, response_frame


class PeerTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.peer = NetPeer(Path(self.directory.name))
        self.peer.start()

    def tearDown(self):
        self.peer.stop()
        self.directory.cleanup()

    def connect(self):
        host, port = self.peer.endpoint.split(":")
        connection = socket.create_connection((host, int(port)), timeout=2)
        connection.settimeout(2)
        return connection

    def receive(self, connection, length):
        result = b""
        while len(result) < length:
            chunk = connection.recv(length - len(result))
            self.assertTrue(chunk)
            result += chunk
        return result

    def test_wrap_exchange_fragmentation_and_receipt(self):
        with self.connect() as connection:
            for sequence in range(PACKETS):
                frame = request_frame(sequence)
                framed = struct.pack("!I", FRAME_SIZE) + frame
                connection.sendall(framed[:2])
                connection.sendall(framed[2:17])
                connection.sendall(framed[17:])
                self.assertEqual(self.receive(connection, 4), struct.pack("!I", FRAME_SIZE))
                reply = self.receive(connection, FRAME_SIZE)
                self.assertEqual(reply, response_frame(frame))
                self.assertNotEqual(reply[14:], frame[14:])
        result = self.peer.stop()
        self.assertTrue(result["ok"], result)
        self.assertEqual(result["bytes"], 13184)
        self.assertIs(self.peer.stop(), result)
        self.assertEqual(json.loads((Path(self.directory.name) / "net-peer.json").read_text()), result)

    def test_no_connection_shutdown(self):
        start = time.monotonic()
        self.assertFalse(self.peer.stop()["ok"])
        self.assertLess(time.monotonic() - start, 1)
        self.assertFalse(self.peer._thread.is_alive())

    def test_oversize_rejected_without_body_allocation(self):
        with self.connect() as connection:
            connection.sendall(struct.pack("!I", 0xFFFFFFFF))
            self.assertEqual(connection.recv(1), b"")
        self.assertIn("invalid frame length", " ".join(self.peer.stop()["errors"]))

    def test_bad_sequence_and_truncated_body(self):
        with self.connect() as connection:
            connection.sendall(struct.pack("!I", FRAME_SIZE) + request_frame(1))
            self.assertEqual(connection.recv(1), b"")
        self.assertIn("sequence", " ".join(self.peer.stop()["errors"]))

    def test_partial_body_shutdown_is_bounded(self):
        with self.connect() as connection:
            connection.sendall(struct.pack("!I", FRAME_SIZE) + b"short")
            self.assertFalse(self.peer.stop()["ok"])
        self.assertFalse(self.peer._thread.is_alive())

    def test_truncated_header(self):
        with self.connect() as connection:
            connection.sendall(b"\0\0")
            connection.shutdown(socket.SHUT_WR)
            self.assertEqual(connection.recv(1), b"")
        self.assertIn("truncated", " ".join(self.peer.stop()["errors"]))


if __name__ == "__main__":
    unittest.main()
