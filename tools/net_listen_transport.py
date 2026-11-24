"""Isolated E1000 peer for application TCP listen/accept and UDP bind."""
from __future__ import annotations

import select
import socket
from threading import Event

from net_dhcp_transport import ARP, BROADCAST_MAC, IPV4
from net_dns_transport import host_tcp, host_udp, parse_tcp, parse_udp
from net_live_transport import ACK, FIN, PSH, SYN, advance
from net_static_transport import arp, ethernet
from net_test_transport import EthernetPeer, GUEST_MAC, HOST_MAC

RST = 0x04

GUEST = bytes((192, 0, 2, 12))
HOST = bytes((192, 0, 2, 2))
CONFLICT_MAC = bytes.fromhex("020000000055")

TCP_PORT = 8080
UDP_PORT = 9090
CLIENT_TCP_PORT = 55111
CLIENT_UDP_PORT = 55222


class NetListenPeer(EthernetPeer):
    """Connects inbound to guest TCP 8080, exchanges bound UDP 9090, and triggers ARP conflict."""

    def __init__(self, workdir):
        super().__init__(workdir)
        self._done = Event()
        self._stages: list[str] = []
        self._announcements = 0

    def wait_complete(self, timeout: float) -> bool:
        expected = ("tcp_accepted", "udp_echo", "conflict_injected")
        return self._done.wait(timeout) and all(s in self._stages for s in expected)

    def _exchange(self, connection: socket.socket) -> None:
        try:
            self._serve(connection)
        finally:
            self._done.set()
        self._stop.wait()

    def _serve(self, connection: socket.socket) -> None:
        tcp_phase = "wait_netd"  # wait_netd -> syn_sent -> established -> fin_sent -> done
        udp_phase = "wait"  # wait -> probing -> done
        host_seq = 0x50000001
        guest_next = 0

        while not self._stop.is_set():
            readable, _, _ = select.select([connection], [], [], 0.4)
            if not readable:
                if tcp_phase in ("wait_netd", "syn_sent") and self._announcements >= 2:
                    self._send(
                        connection,
                        host_tcp(HOST, CLIENT_TCP_PORT, TCP_PORT, host_seq - 1, 0, SYN),
                    )
                    tcp_phase = "syn_sent"
                elif tcp_phase == "done" and udp_phase in ("wait", "probing"):
                    self._send(
                        connection,
                        host_udp(HOST, CLIENT_UDP_PORT, UDP_PORT, b"udp-listen-in"),
                    )
                    udp_phase = "probing"
                continue

            frame = self._receive(connection)
            eth = frame[12:14]
            if eth == ARP:
                if frame == ethernet(BROADCAST_MAC, GUEST_MAC, ARP, arp(1, GUEST_MAC, bytes(4), bytes(6), GUEST)):
                    continue
                if frame == ethernet(BROADCAST_MAC, GUEST_MAC, ARP, arp(1, GUEST_MAC, GUEST, bytes(6), GUEST)):
                    self._announcements += 1
                    continue
                if frame == ethernet(BROADCAST_MAC, GUEST_MAC, ARP, arp(1, GUEST_MAC, GUEST, bytes(6), HOST)):
                    self._send(connection, ethernet(GUEST_MAC, HOST_MAC, ARP, arp(2, HOST_MAC, HOST, GUEST_MAC, GUEST)))
                continue

            if eth != IPV4:
                continue
            proto = frame[23]
            if proto == 6:
                dst, sport, dport, seq, ack, flags, data = parse_tcp(frame)
                if dst != HOST or sport != TCP_PORT or dport != CLIENT_TCP_PORT:
                    raise ValueError(f"unexpected TCP dst={dst!r} sport={sport} dport={dport}")
                if flags & RST:
                    raise ValueError("unexpected TCP RST from listener")
                if tcp_phase in ("wait_netd", "syn_sent") and (flags & (SYN | ACK)) == (SYN | ACK):
                    guest_next = advance(seq, 1)
                    self._send(
                        connection,
                        host_tcp(HOST, CLIENT_TCP_PORT, TCP_PORT, host_seq, guest_next, ACK),
                    )
                    self._send(
                        connection,
                        host_tcp(HOST, CLIENT_TCP_PORT, TCP_PORT, host_seq, guest_next, PSH | ACK, b"hello-listen"),
                    )
                    host_seq = advance(host_seq, 12)
                    tcp_phase = "established"
                elif tcp_phase == "established":
                    if data:
                        if data != b"listen-ack":
                            raise ValueError(f"unexpected TCP payload {data!r}")
                        guest_next = advance(guest_next, len(data))
                    if flags & FIN:
                        guest_next = advance(guest_next, 1)
                        self._send(
                            connection,
                            host_tcp(HOST, CLIENT_TCP_PORT, TCP_PORT, host_seq, guest_next, FIN | ACK),
                        )
                        host_seq = advance(host_seq, 1)
                        tcp_phase = "fin_sent"
                    elif data:
                        self._send(
                            connection,
                            host_tcp(HOST, CLIENT_TCP_PORT, TCP_PORT, host_seq, guest_next, ACK),
                        )
                elif tcp_phase == "fin_sent" and (flags & ACK):
                    self._stages.append("tcp_accepted")
                    tcp_phase = "done"
                    self._send(
                        connection,
                        host_udp(HOST, CLIENT_UDP_PORT, UDP_PORT, b"udp-listen-in"),
                    )
                    udp_phase = "probing"
            elif proto == 17:
                dst, sport, dport, data = parse_udp(frame)
                if dst != HOST or sport != UDP_PORT or dport != CLIENT_UDP_PORT:
                    raise ValueError(f"unexpected UDP dst={dst!r} sport={sport} dport={dport}")
                if data != b"udp-listen-out":
                    raise ValueError(f"unexpected UDP reply {data!r}")
                self._stages.append("udp_echo")
                udp_phase = "done"
                self._send(
                    connection,
                    ethernet(BROADCAST_MAC, CONFLICT_MAC, ARP, arp(1, CONFLICT_MAC, GUEST, bytes(6), GUEST)),
                )
                self._stages.append("conflict_injected")
                return

    def _finish_exchange(self) -> dict:
        return {
            "protocols": ["ARP", "TCP", "UDP"],
            "stages": self._stages,
        }
