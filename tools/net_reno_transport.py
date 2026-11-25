"""Isolated E1000 peer verifying TCP Reno slow start, pipelining, and 3-dupACK fast retransmit."""
from __future__ import annotations

import socket
import time
from threading import Event

from net_dhcp_transport import ARP, BROADCAST_MAC, IPV4
from net_dns_transport import parse_tcp
from net_live_transport import ACK, FIN, PSH, SYN, advance
from net_static_transport import arp, ethernet
from net_test_transport import EthernetPeer, GUEST_MAC, HOST_MAC

RST = 0x04

GUEST = bytes((192, 0, 2, 12))
HOST = bytes((192, 0, 2, 2))
CONFLICT_MAC = bytes.fromhex("020000000066")
TCP_PORT = 5432
MSS = 1460
TOTAL_BYTES = 16 * MSS


def host_tcp_win(src_port: int, dst_port: int, seq: int, ack: int, flags: int, data: bytes = b"") -> bytes:
    import struct
    from net_live_transport import pseudo
    from net_test_transport import checksum
    from net_dhcp_transport import ipv4

    opts = struct.pack("!BBH", 2, 4, MSS) if (flags & SYN) else b""
    off = (5 + len(opts) // 4) << 4
    hdr = struct.pack("!HHIIBBHHH", src_port, dst_port, seq, ack, off, flags, 16384, 0, 0) + opts
    seg = hdr + data
    chk = checksum(pseudo(HOST, GUEST, 6, len(seg)) + seg)
    ip = ipv4(6, HOST, GUEST, seg[:16] + struct.pack("!H", chk) + seg[18:])
    return ethernet(GUEST_MAC, HOST_MAC, IPV4, ip)


class NetRenoPeer(EthernetPeer):
    """Drops one segment after slow-start growth and requires fast retransmit (< 800ms)."""

    def __init__(self, workdir):
        super().__init__(workdir)
        self._done = Event()
        self._stages: list[str] = []

    def wait_complete(self, timeout: float) -> bool:
        expected = ("fast_retransmit_ok", "total_bytes_ok", "conflict_injected")
        return self._done.wait(timeout) and all(s in self._stages for s in expected)

    def _exchange(self, connection: socket.socket) -> None:
        try:
            self._serve(connection)
        finally:
            self._done.set()
        self._stop.wait()

    def _serve(self, connection: socket.socket) -> None:
        host_seq = 0x80000001
        sport = None
        base_seq = None
        recv_next = 0
        ooo: dict[int, bytes] = {}
        assembled = bytearray()
        data_pkts = 0
        drop_seq = None
        dropped = False
        drop_time = 0.0
        fast_retransmitted = False
        fin_seq = None

        while True:
            frame = self._receive(connection)
            eth = frame[12:14]
            if eth == ARP:
                if frame == ethernet(BROADCAST_MAC, GUEST_MAC, ARP, arp(1, GUEST_MAC, bytes(4), bytes(6), GUEST)):
                    continue
                if frame == ethernet(BROADCAST_MAC, GUEST_MAC, ARP, arp(1, GUEST_MAC, GUEST, bytes(6), GUEST)):
                    continue
                if frame == ethernet(BROADCAST_MAC, GUEST_MAC, ARP, arp(1, GUEST_MAC, GUEST, bytes(6), HOST)):
                    self._send(connection, ethernet(GUEST_MAC, HOST_MAC, ARP, arp(2, HOST_MAC, HOST, GUEST_MAC, GUEST)))
                continue
            if eth != IPV4 or frame[23] != 6:
                continue

            dst, g_port, dport, seq, ack, flags, data = parse_tcp(frame)
            if dst != HOST or dport != TCP_PORT:
                raise ValueError(f"unexpected TCP dst={dst!r} dport={dport}")
            if flags & RST:
                raise ValueError("unexpected TCP RST during Reno exchange")

            if flags & SYN:
                if "total_bytes_ok" in self._stages:
                    self._send(
                        connection,
                        ethernet(BROADCAST_MAC, CONFLICT_MAC, ARP, arp(1, CONFLICT_MAC, GUEST, bytes(6), GUEST)),
                    )
                    self._stages.append("conflict_injected")
                    return
                sport = g_port
                base_seq = advance(seq, 1)
                recv_next = base_seq
                self._send(connection, host_tcp_win(TCP_PORT, sport, host_seq - 1, recv_next, SYN | ACK))
                continue

            if sport is None or g_port != sport:
                continue

            if data:
                data_pkts += 1
                if not dropped and data_pkts == 5 and seq == recv_next:
                    drop_seq = seq
                    dropped = True
                    drop_time = time.monotonic()
                    continue
                if dropped and seq == drop_seq and not fast_retransmitted:
                    elapsed_ms = (time.monotonic() - drop_time) * 1000.0
                    if elapsed_ms >= 850.0:
                        raise ValueError(f"retransmit took {elapsed_ms:.1f}ms (expected fast retransmit < RTO)")
                    fast_retransmitted = True
                    self._stages.append("fast_retransmit_ok")

                if seq == recv_next:
                    assembled.extend(data)
                    recv_next = advance(recv_next, len(data))
                    while recv_next in ooo:
                        chunk = ooo.pop(recv_next)
                        assembled.extend(chunk)
                        recv_next = advance(recv_next, len(chunk))
                elif (seq - recv_next) & 0xFFFFFFFF < 0x80000000:
                    ooo.setdefault(seq, data)
                self._send(connection, host_tcp_win(TCP_PORT, sport, host_seq, recv_next, ACK))

            if flags & FIN:
                fin_seq = advance(seq, len(data))
            if fin_seq is not None and recv_next == fin_seq:
                expected_bytes = bytes((((i * 31) ^ (i >> 8)) & 255) for i in range(TOTAL_BYTES))
                if bytes(assembled) != expected_bytes:
                    raise ValueError(f"payload mismatch: got {len(assembled)} bytes")
                self._stages.append("total_bytes_ok")
                recv_next = advance(recv_next, 1)
                reply = b"reno-23360-ok"
                self._send(connection, host_tcp_win(TCP_PORT, sport, host_seq, recv_next, PSH | ACK | FIN, reply))
                host_seq = advance(host_seq, len(reply) + 1)
                fin_seq = None
                continue

    def _finish_exchange(self) -> dict:
        return {
            "protocols": ["ARP", "TCP"],
            "stages": self._stages,
        }

