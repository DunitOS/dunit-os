# IPv4 configuration and reference transports

The DWM image's `netd` uses the sealed boot catalog and the existing native
packet queue. It prefers a static IPv4 address under
`/system/boot-services/netd-ipv4.conf` in address/prefix form. With that file
absent, netd negotiates an IPv4 lease with a directly connected DHCP server.
Other applications keep their existing authority.

DHCP validates the transaction ID, interface MAC, selected server, address,
subnet, gateway, DNS servers and lifetime before activating a lease. The
service constructs the new interface and connected/default routes before
publication. An unchanged renewal retains the peer and its ARP cache. Expiry,
link loss or an accepted NAK clears the addresses and transmit backlog.
The service waits on packet readiness and monotonic deadlines. It bounds TX
backlog to one batch of 32 records so a slow transmitter cannot stall RX or
lease transitions. The DHCP client uses a local noncryptographic transaction
seed and bounded retries.

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
python3 tools/qemu_test.py --build --markers-file tools/net_live_markers.json \
  --cpus 8 --accel tcg --timeout 250
```

The DHCP host peer verifies retry after loss, unrelated OFFER rejection,
unicast renewal, broadcast rebinding, NAK, and ARP/ICMP on the replacement
address. The retired address is silent. The separate native logical test
covers 8,192 bytes of TCP with losses and both half-closes. Its host counterpart
also covers 2 MiB, sequence wrap and timer bounds.

Application sockets, production netd integration for TCP and DNS, congestion
control, DHCP relays, address-conflict probes and persisted lease recovery
remain open. See the [IPv4 verification](../verification/net-stack-2026-11-19.json)
and [TCP admission verification](../verification/tcp-admission-2026-11-19.json).
