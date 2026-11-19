"""Strict localhost Ethernet peer for the supervised native packet queue fixture."""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import socket
import struct

from net_test_transport import GUEST_MAC, HOST_MAC, EthernetPeer, checksum

GUEST = bytes((192, 0, 2, 2))
HOST = bytes((192, 0, 2, 1))
SECONDARY = bytes((192, 0, 2, 53))
ARP = b"\x08\x06"
IPV4 = b"\x08\x00"
ETH_BROADCAST = b"\xff" * 6
DNS_QUESTION = b"\x02du\x03net\x04test\x00\x00\x01\x00\x01"
TCP_QUESTION = b"\x03tcp" + DNS_QUESTION
MISSING_QUESTION = b"\x07missing" + DNS_QUESTION
FAILOVER_QUESTION = b"\x08failover" + DNS_QUESTION
DNS_PORT = 53
LISTENER_PORT = 8080
SYN, ACK, PSH, FIN = 0x02, 0x10, 0x08, 0x01
ECHO_DATA = b"dunit-netd-live"


def ether(kind: bytes, payload: bytes) -> bytes:
    return (GUEST_MAC + HOST_MAC + kind + payload).ljust(60, b"\0")


def ipv4(protocol: int, payload: bytes, *, source: bytes = HOST) -> bytes:
    header = struct.pack("!BBHHHBBH4s4s", 0x45, 0, 20 + len(payload), 0,
                         0x4000, 64, protocol, 0, source, GUEST)
    header = header[:10] + struct.pack("!H", checksum(header)) + header[12:]
    return ether(IPV4, header + payload)


def guest_ipv4(frame: bytes, protocol: int, *, target: bytes = HOST) -> bytes:
    if frame[:14] != HOST_MAC + GUEST_MAC + IPV4:
        raise ValueError("expected unicast guest IPv4")
    header = frame[14:34]
    total = int.from_bytes(header[2:4], "big")
    if (header[0] != 0x45 or not 20 <= total <= len(frame) - 14 or
            header[9] != protocol or header[12:20] != GUEST + target or checksum(header)):
        raise ValueError("invalid guest IPv4 header")
    return frame[34:14 + total]


def echo_response(frame: bytes) -> tuple[bytes, int]:
    echo = guest_ipv4(frame, 1)
    if echo[:2] != b"\x08\x00" or echo[6:8] != b"\x00\x01" or echo[8:] != ECHO_DATA or checksum(echo):
        raise ValueError("invalid guest ICMP echo")
    reply = b"\0\0\0\0" + echo[4:]
    reply = reply[:2] + struct.pack("!H", checksum(reply)) + reply[4:]
    return ipv4(1, reply), int.from_bytes(echo[4:6], "big")


def pseudo(source: bytes, destination: bytes, protocol: int, length: int) -> bytes:
    return struct.pack("!4s4sBBH", source, destination, 0, protocol, length)


def dns_answer(dns: bytes, question: bytes, answer_octet: int, truncated: bool) -> bytes:
    if dns[2:12] != struct.pack("!5H", 0x0100, 1, 0, 0, 0) or dns[12:] != question:
        raise ValueError("invalid guest DNS question")
    flags = 0x8380 if truncated else 0x8180
    wire = dns[:2] + struct.pack("!5H", flags, 1, int(not truncated), 0, 0) + question
    if truncated:
        return wire
    return wire + struct.pack("!HHHIH4B", 0xc00c, 1, 1, 30, 4, 192, 0, 2, answer_octet)


def dns_response(frame: bytes, answer_octet: int, *, truncated: bool = False,
                 negative: bool = False, failover: bool = False, server: bytes = HOST) -> bytes:
    udp = guest_ipv4(frame, 17, target=server)
    if len(udp) < 8:
        raise ValueError("truncated UDP header")
    source, destination, length, check = struct.unpack("!HHHH", udp[:8])
    if (not source or destination != 53 or length != len(udp) or not check or
            checksum(pseudo(GUEST, server, 17, length) + udp)):
        raise ValueError("invalid DNS transport")
    if negative:
        query = udp[8:]
        if query[2:12] != struct.pack("!5H", 0x0100, 1, 0, 0, 0) or query[12:] != MISSING_QUESTION:
            raise ValueError("invalid negative-cache DNS question")
        zone = 0xc014  # "du.net.test", after the first 8-byte label
        soa = struct.pack("!HHHIHHH5I", zone, 6, 1, 30, 24, zone, zone, 1, 60, 60, 60, 10)
        opt = struct.pack("!BHHIH", 0, 41, 512, 0, 0)
        dns_reply = query[:2] + struct.pack("!5H", 0x8183, 1, 0, 1, 1) + MISSING_QUESTION + soa + opt
    elif failover:
        dns_reply = dns_answer(udp[8:], FAILOVER_QUESTION, answer_octet, False)
        if server == HOST:
            dns_reply = dns_reply[:2] + struct.pack("!5H", 0x8182, 1, 0, 0, 0) + FAILOVER_QUESTION
    else:
        question = TCP_QUESTION if truncated else DNS_QUESTION
        dns_reply = dns_answer(udp[8:], question, answer_octet, truncated)
    udp_length = 8 + len(dns_reply)
    reply = struct.pack("!HHHH", 53, source, udp_length, 0) + dns_reply
    udp_checksum = checksum(pseudo(server, GUEST, 17, udp_length) + reply) or 0xffff
    return ipv4(17, reply[:6] + struct.pack("!H", udp_checksum) + reply[8:], source=server)


@dataclass(frozen=True)
class TcpSegment:
    source: int
    destination: int
    sequence: int
    acknowledgement: int
    flags: int
    data: bytes

    def contents(self) -> tuple[int, int, int, bytes]:
        return self.sequence, self.acknowledgement, self.flags, self.data


def guest_tcp(frame: bytes) -> TcpSegment:
    segment = guest_ipv4(frame, 6)
    if len(segment) < 20:
        raise ValueError("truncated TCP header")
    source, destination, seq, ack, offset, flags, _, _, urgent = struct.unpack(
        "!HHIIBBHHH", segment[:20])
    size = (offset >> 4) * 4
    if (not source or not destination or offset & 15 or urgent or
            not 20 <= size <= len(segment) or
            checksum(pseudo(GUEST, HOST, 6, len(segment)) + segment)):
        raise ValueError("invalid guest TCP segment")
    return TcpSegment(source, destination, seq, ack, flags, segment[size:])


def host_tcp(source: int, destination: int, seq: int, ack: int, flags: int,
             data: bytes = b"") -> bytes:
    header = struct.pack("!HHIIBBHHH", source, destination, seq, ack, 0x50, flags, 4096, 0, 0)
    segment = header + data
    check = checksum(pseudo(HOST, GUEST, 6, len(segment)) + segment)
    return ipv4(6, segment[:16] + struct.pack("!H", check) + segment[18:])


def advance(sequence: int, count: int) -> int:
    return (sequence + count) & 0xffffffff


@dataclass
class PassiveStream:
    host_next: int
    message: bytes
    syn_ack: bytes | None = None
    guest_next: int = 0
    echoed: int = 0
    closed: bool = False
    empty_ack: bool = False


class NetLivePeer(EthernetPeer):
    """Verify two complete protocol exchanges across the supervised restart."""

    def __init__(self, workdir: Path):
        super().__init__(workdir)
        self._identifiers: list[int] = []
        self._tcp_rounds = 0
        self._passive_streams = 0
        self._passive_packets = 0
        self._passive_responses = 0

    def _exchange(self, connection: socket.socket) -> None:
        for answer_octet in (3, 4):
            frame = self._receive(connection)
            arp_prefix = struct.pack("!HHBB", 1, 0x0800, 6, 4)
            request = (ETH_BROADCAST + GUEST_MAC + ARP + arp_prefix + b"\0\x01" +
                       GUEST_MAC + GUEST + b"\0" * 6 + HOST).ljust(60, b"\0")
            if frame != request:
                raise ValueError("invalid guest ARP request")
            self._send(connection, ether(ARP, arp_prefix + b"\0\x02" +
                                        HOST_MAC + HOST + GUEST_MAC + GUEST))
            reply, identifier = echo_response(self._receive(connection))
            if identifier in self._identifiers:
                raise ValueError("ICMP exchange did not move to a fresh service process")
            self._identifiers.append(identifier)
            self._send(connection, reply)
            self._send(connection, dns_response(self._receive(connection), answer_octet))
            self._send(connection, dns_response(self._receive(connection), answer_octet, negative=True))
            primary = self._receive(connection)
            self._send(connection, dns_response(primary, answer_octet, failover=True))
            request = (ETH_BROADCAST + GUEST_MAC + ARP + arp_prefix + b"\0\x01" +
                       GUEST_MAC + GUEST + b"\0" * 6 + SECONDARY).ljust(60, b"\0")
            if self._receive(connection) != request:
                raise ValueError("expected ARP for secondary DNS server")
            self._send(connection, ether(ARP, arp_prefix + b"\0\x02" +
                                        HOST_MAC + SECONDARY + GUEST_MAC + GUEST))
            secondary = self._receive(connection)
            if guest_ipv4(secondary, 17, target=SECONDARY)[8:10] == guest_ipv4(primary, 17)[8:10]:
                raise ValueError("DNS failover reused the primary transaction ID")
            self._send(connection, dns_response(secondary, answer_octet, failover=True, server=SECONDARY))
            self._send(connection, dns_response(self._receive(connection), answer_octet, truncated=True))
            self._tcp_dns(connection, answer_octet)
            packets, responses = self._packets, self._responses
            self._tcp_passive(connection, answer_octet)
            self._passive_packets += self._packets - packets
            self._passive_responses += self._responses - responses

    def _tcp_dns(self, connection: socket.socket, answer_octet: int) -> None:
        first = self._receive(connection)  # Deliberately lose the first SYN.
        syn = guest_tcp(first)
        if (not 49152 <= syn.source <= 65535 or syn.destination != DNS_PORT or
                (syn.acknowledgement, syn.flags, syn.data) != (0, SYN, b"") or
                self._receive(connection) != first):
            raise ValueError("missing identical retransmit after dropped TCP SYN")
        guest_next = advance(syn.sequence, 1)
        client_port = syn.source
        host_seq = 0x32000000 + answer_octet
        self._send(connection, host_tcp(DNS_PORT, client_port, host_seq, guest_next, SYN | ACK))
        host_seq += 1

        def receive() -> tuple[int, int, int, bytes]:
            segment = guest_tcp(self._receive(connection))
            if (segment.source, segment.destination) != (client_port, DNS_PORT):
                raise ValueError("DNS TCP tuple changed within a stream")
            return segment.contents()

        seq, ack, flags, data = receive()
        if ((seq, ack, flags) != (guest_next, host_seq, PSH | ACK) or
                len(data) < 2 or int.from_bytes(data[:2], "big") != len(data) - 2):
            raise ValueError("invalid TCP DNS query or SYN acknowledgement")
        guest_next = advance(guest_next, len(data))
        response = dns_answer(data[2:], TCP_QUESTION, answer_octet, False)
        framed = struct.pack("!H", len(response)) + response
        self._send(connection, host_tcp(DNS_PORT, client_port, host_seq, guest_next, PSH | ACK, framed[:1]))
        host_seq += 1
        if receive() != (guest_next, host_seq, ACK, b""):
            raise ValueError("client did not acknowledge first TCP length byte")

        self._send(connection, host_tcp(DNS_PORT, client_port, host_seq, guest_next, PSH | ACK | FIN, framed[1:]))
        host_seq += len(framed)
        if receive() != (guest_next, host_seq, ACK | FIN, b""):
            raise ValueError("client did not finish its half of the TCP stream")
        self._send(connection, host_tcp(DNS_PORT, client_port, host_seq, advance(guest_next, 1), ACK))
        self._tcp_rounds += 1

    def _tcp_passive(self, connection: socket.socket, round_number: int) -> None:
        streams = {
            49201: PassiveStream(0x33000000 + round_number, b"tcp-passive-first"),
            49202: PassiveStream(0x34000000 + round_number, b"tcp-passive-second"),
        }

        def receive() -> tuple[int, PassiveStream, TcpSegment, bytes]:
            frame = self._receive(connection)
            segment = guest_tcp(frame)
            stream = streams.get(segment.destination)
            if segment.source != LISTENER_PORT or stream is None:
                raise ValueError("unexpected passive TCP tuple")
            return segment.destination, stream, segment, frame

        # Send both SYNs before acknowledging either. Only the second handshake
        # completes initially. Drop the first stream's SYN-ACK and require its
        # retransmission before either stream carries application data.
        for port, stream in streams.items():
            self._send(connection, host_tcp(port, LISTENER_PORT, stream.host_next, 0, SYN))
            stream.host_next += 1
        # A third simultaneous peer exceeds the fixture listener's backlog.
        # Every subsequent receive checks that only the two admitted tuples respond.
        self._send(connection, host_tcp(49203, LISTENER_PORT, 0x24000000, 0, SYN))
        for _ in streams:
            port, stream, segment, frame = receive()
            if (stream.syn_ack is not None or segment.flags != (SYN | ACK) or
                    segment.acknowledgement != stream.host_next or segment.data):
                raise ValueError("invalid or duplicate initial passive TCP SYN-ACK")
            stream.syn_ack = frame
            stream.guest_next = advance(segment.sequence, 1)
            if port == 49202:
                self._send(connection, host_tcp(port, LISTENER_PORT, stream.host_next, stream.guest_next, ACK))

        port, stream, _, frame = receive()
        if port != 49201 or frame != stream.syn_ack:
            raise ValueError("expected identical retransmit of the first passive TCP SYN-ACK")
        self._send(connection, host_tcp(port, LISTENER_PORT, stream.host_next, stream.guest_next, ACK))

        for port, stream in streams.items():
            self._send(connection, host_tcp(port, LISTENER_PORT, stream.host_next,
                                            stream.guest_next, PSH | ACK | FIN, stream.message))
            stream.host_next += len(stream.message) + 1

        # Each tiny message fits below the minimum MSS. Allow one cumulative
        # pure ACK per flow and verify every echoed byte and the final FIN.
        for _ in range(8):
            port, stream, segment, _ = receive()
            seq, ack, flags, data = segment.contents()
            if (stream.closed or seq != stream.guest_next or ack != stream.host_next or
                    flags & ~(PSH | ACK | FIN) or not flags & ACK):
                raise ValueError("invalid sequence, acknowledgment or flags in passive echo")
            if not data and not flags & FIN:
                if flags != ACK or stream.empty_ack:
                    raise ValueError("unexpected pure ACK in passive echo")
                stream.empty_ack = True
                continue
            end = stream.echoed + len(data)
            if data != stream.message[stream.echoed:end]:
                raise ValueError("passive TCP echo crossed streams or changed bytes")
            stream.echoed = end
            stream.guest_next = advance(stream.guest_next, len(data))
            if flags & FIN:
                if stream.echoed != len(stream.message):
                    raise ValueError("guest closed write side before echoing the message")
                stream.closed = True
                stream.guest_next = advance(stream.guest_next, 1)
                self._passive_streams += 1
            self._send(connection, host_tcp(port, LISTENER_PORT, stream.host_next,
                                            stream.guest_next, ACK))
            if all(item.closed for item in streams.values()):
                return
        raise ValueError("passive TCP streams did not finish within the frame budget")

    def _finish_exchange(self) -> dict:
        baseline = self._packets - self._passive_packets, self._responses - self._passive_responses
        if (*baseline, len(self._identifiers), self._tcp_rounds, self._passive_streams) != (26, 24, 2, 2, 4):
            self.errors.append("incomplete ARP/ICMP/UDP/TCP DNS/passive TCP and restart exchange")
        return {"protocols": ["ARP", "ICMPv4", "UDP", "TCP", "DNS"],
                "guest_identifiers": self._identifiers, "tcp_rounds": self._tcp_rounds,
                "passive_streams": self._passive_streams,
                "passive_packets": self._passive_packets}
