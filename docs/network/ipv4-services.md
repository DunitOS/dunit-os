# IPv4 configuration and reference transports

The DWM image's `netd` uses the sealed boot catalog and the existing native
packet queue. It prefers a static IPv4 address under
`/system/boot-services/netd-ipv4.conf`:

```text
192.0.2.12/24
gateway=192.0.2.1
dns=192.0.2.53,192.0.2.54
```

The address/prefix line is required. Gateway and one or two DNS servers are
optional. Blank lines and comment lines starting with `#` are allowed.
Netd validates the whole file and connected gateway before advertising the
address. Static mode probes and announces the configured address, withdraws it
on conflict, and retries on a detected link down/up or reboot. With the file
absent, netd negotiates an IPv4 lease directly or through a DHCPv4 relay.
The configured DNS addresses are retained with the interface state. The
application resolver service remains separate.

DHCP validates the transaction ID, interface MAC, selected server, address,
subnet, gateway, DNS servers and lifetime before activating a lease. The
service waits for three RFC 5227 ARP probes and a quiet interval before
activating a new DHCP address. It then sends two announcements and answers
foreign probes for the configured address. The schedule advances when the
packet queue accepts a probe or announcement. An ARP conflict withdraws the
lease immediately and sends DHCPDECLINE, followed by a ten-second cooldown
before rediscovery. An unchanged renewal retains the peer and its ARP cache.
Expiry,
link loss or an accepted NAK clears the addresses and transmit backlog.
The service waits on packet readiness and monotonic deadlines. It bounds TX
backlog to one batch of 32 records so a slow transmitter cannot stall RX or
lease transitions. The DHCP client uses a local noncryptographic transaction
seed and bounded retries.

For forwarded replies netd accepts the server's IP or the relay's BOOTP
`giaddr` as the source. The DHCP Server Identifier (option 54) still selects
the server and supplies the unicast renewal destination. An off-link server
requires a matching route in the lease for unicast renewal. If renewal
fails, rebinding uses broadcast as before.

When static IPv4 is absent, the boot archive can supply one previously used
IPv4 address in `/system/boot-services/netd-dhcp-address.conf`. Netd tries it
once at startup using DHCP INIT-REBOOT. It broadcasts DHCPREQUEST from
`0.0.0.0` with the Requested IP Address option and without a Server
Identifier. Only a fresh ACK for that address supplies interface settings.
The same ARP probing and conflict handling run before activation. A NAK or
no response after four requests returns the client to discovery. Automatic
on-disk storage of the address hint remains open.

DHCP option 121 carries up to seven classless routes. A nonzero gateway
must be reachable in the primary connected subnet. A zero gateway marks
the prefix as on-link, so outbound traffic resolves the destination itself
with ARP. Longest-prefix matching applies to both kinds of route. A present
classless list takes precedence over Router option 3, including when the list
supplies no default route. A same-server renewal can omit the list and retain
the previous routes. Invalid lists are rejected before publication. The DHCP
peer checks routed and direct delivery, retention on renewal, and loss of
direct delivery when the replacement lease omits that route.

The reference TCP connection owns one four-tuple, a 4 KiB send queue, a
4 KiB receive queue and one segment in flight. It supports active and passive
open, retransmission, cumulative/partial ACKs, zero-window probes, FIN/RST and
TIME_WAIT. Out-of-order gaps are acknowledged cumulatively and discarded.

The reference connection table demultiplexes up to eight streams and four
listeners. Each listener counts half-open and unaccepted connections against
its backlog. At most four connections may be pending across all listeners.
Pending and detached states expire with bounded lifetimes. Application
readiness reports queued data or EOF, write capacity, hangup and errors.
Generations invalidate IDs on close, slot reuse and interface replacement.
The native fixture uses the table for both DNS and two concurrent incoming
streams per supervised instance. The peer checks retransmitted SYN-ACK,
backlog refusal, correct per-stream data, and FIN completion.

An opt-in DNS resolver recognizes a validated truncated UDP reply and emits one
length-prefixed query for the same server. The caller owns the TCP connection.
The resolver accepts ordered stream bytes up to 4 KiB, checks the complete
transaction, and retains the validated answer in the existing TTL cache.
The native fixture deliberately drops the first SYN and sends the DNS length
prefix across two TCP segments. It validates the response and close twice,
before and after a supervised restart. The UDP-only API stays available.

## Verification

```sh
python3 tools/test_net_core.py
python3 tools/qemu_test.py --build --image-profile dwm --config limine_dwm.conf \
  --markers-file tools/net_dhcp_markers.json --cpus 4 --accel tcg --timeout 150
python3 tools/qemu_test.py --build --image-profile dwm --config limine_dwm.conf \
  --markers-file tools/net_static_markers.json --cpus 4 --accel tcg --timeout 140
python3 tools/qemu_test.py --build --markers-file tools/net_live_markers.json \
  --cpus 8 --accel tcg --timeout 250
```

The DHCP host peer verifies retry after loss, ARP probing after an ACK for
a remembered address, unrelated OFFER rejection,
lease acquisition through a relay, routed unicast renewal, broadcast
rebinding, relayed NAK, and ARP/ICMP on the replacement address. The retired
address is silent. The separate native logical test
covers 8,192 bytes of TCP with losses and both half-closes. Its host counterpart
also covers 2 MiB, sequence wrap and timer bounds.

Stock netd also handles [explicitly granted outbound IPv4 sockets](application-sockets.md).
Application DNS, TCP listener authority, production congestion control and
persisted lease recovery remain open. See [IPv4 verification](../verification/net-stack-2026-11-19.json),
[TCP admission verification](../verification/tcp-admission-2026-11-19.json),
[DHCP conflict verification](../verification/dhcp-acd-2026-11-19.json), and
[static and gateway route verification](../verification/ipv4-routing-2026-11-19.json).
See [on-link DHCP verification](../verification/dhcp-on-link-2026-11-19.json)
and [relayed DHCP verification](../verification/dhcp-relay-2026-11-19.json).
The startup request and conflict checks are recorded in
[INIT-REBOOT verification](../verification/dhcp-init-reboot-2026-11-19.json).
