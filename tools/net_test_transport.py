"""Bounded localhost QEMU socket-net peer for the private 82540EM INTx probe.

QEMU frames Ethernet packets with a four-byte big-endian length. The peer accepts
exactly 103 deterministic frames, swaps MACs and XORs every payload byte with A5.
There is no host NIC, Internet, TAP privilege or third-party service involved.
"""
from __future__ import annotations

import json
from pathlib import Path
import socket
import struct
import threading

PACKETS = 103
FRAME_SIZE = 128
GUEST_MAC = bytes.fromhex("525400123456")
HOST_MAC = bytes.fromhex("020000000001")
ETHERTYPE = bytes.fromhex("88b5")


def request_frame(sequence: int) -> bytes:
    return HOST_MAC + GUEST_MAC + ETHERTYPE + bytes(
        (sequence * 37 + index) & 255 for index in range(FRAME_SIZE - 14)
    )


def response_frame(frame: bytes) -> bytes:
    return frame[6:12] + frame[:6] + frame[12:14] + bytes(byte ^ 0xA5 for byte in frame[14:])


class NetPeer:
    """One-use peer; start returns only after the listening socket is ready."""

    def __init__(self, workdir: Path):
        self.workdir = Path(workdir)
        self.errors: list[str] = []
        self.endpoint = ""
        self._listener: socket.socket | None = None
        self._connection: socket.socket | None = None
        self._thread: threading.Thread | None = None
        self._stop = threading.Event()
        self._result: dict | None = None
        self._packets = 0
        self._bytes = 0
        self._responses = 0

    def start(self) -> None:
        if self._thread is not None or self._result is not None:
            raise RuntimeError("network peer is one-use")
        listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            listener.bind(("127.0.0.1", 0))
            listener.listen(1)
            listener.settimeout(0.2)
        except BaseException:
            listener.close()
            raise
        self._listener = listener
        self.endpoint = f"127.0.0.1:{listener.getsockname()[1]}"
        self._thread = threading.Thread(target=self._run, name="dunit-net-peer", daemon=True)
        self._thread.start()

    def _read(self, connection: socket.socket, length: int) -> bytes | None:
        data = bytearray()
        while len(data) < length and not self._stop.is_set():
            try:
                chunk = connection.recv(length - len(data))
            except socket.timeout:
                continue
            if not chunk:
                if data:
                    raise ValueError("truncated socket frame")
                return None
            data.extend(chunk)
        if len(data) != length:
            return None
        return bytes(data)

    def _run(self) -> None:
        try:
            while not self._stop.is_set():
                try:
                    connection, address = self._listener.accept()
                    break
                except socket.timeout:
                    continue
            else:
                return
            if address[0] != "127.0.0.1":
                connection.close()
                raise ValueError("non-local peer rejected")
            self._connection = connection
            with connection:
                connection.settimeout(0.2)
                while not self._stop.is_set():
                    header = self._read(connection, 4)
                    if header is None:
                        return
                    length = struct.unpack("!I", header)[0]
                    if length != FRAME_SIZE:
                        raise ValueError(f"invalid frame length {length}; expected {FRAME_SIZE}")
                    frame = self._read(connection, length)
                    if frame is None:
                        if not self._stop.is_set():
                            raise ValueError("missing frame body")
                        return
                    if self._packets >= PACKETS:
                        raise ValueError("unexpected extra frame")
                    if frame != request_frame(self._packets):
                        raise ValueError(f"invalid MAC/EtherType/sequence/payload at frame {self._packets}")
                    self._packets += 1
                    self._bytes += length
                    response = response_frame(frame)
                    connection.sendall(struct.pack("!I", len(response)) + response)
                    self._responses += 1
        except (OSError, ValueError) as error:
            if not self._stop.is_set():
                self.errors.append(str(error))

    def stop(self) -> dict:
        if self._result is not None:
            return self._result
        self._stop.set()
        # Shutdown interrupts blocked reads/sendall. The listener uses a bounded
        # accept timeout, so no connection is also a prompt, joinable shutdown.
        if self._connection is not None:
            try:
                self._connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        if self._thread is not None:
            self._thread.join(timeout=2)
            if self._thread.is_alive():
                self.errors.append("peer thread did not stop")
        if self._listener is not None:
            self._listener.close()
        if self._packets != PACKETS or self._responses != PACKETS:
            self.errors.append(f"incomplete exchange: received={self._packets} replied={self._responses} expected={PACKETS}")
        self._result = {"ok": not self.errors, "packets": self._packets,
                        "bytes": self._bytes, "responses": self._responses,
                        "errors": list(self.errors), "transport": "localhost-tcp-qemu-socket",
                        "ethertype": "0x88b5", "payload_transform": "xor-0xa5"}
        self.workdir.mkdir(parents=True, exist_ok=True)
        (self.workdir / "net-peer.json").write_text(json.dumps(self._result, indent=2) + "\n")
        return self._result
