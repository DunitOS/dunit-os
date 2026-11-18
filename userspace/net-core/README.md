# Userspace packet core (N1/N2 logical foundation)

`dunit-net-core` is a dependency-free `no_std` + `alloc` library shared by the
native SDK `net_packet_test` app and host tests. It has no syscall, kernel,
MMIO, DMA, device handle or service dependency. `Peer` exchanges owned frame
vectors with a logical peer supplied by its caller. It does not open an
interface, send packets to a NIC, or provide Internet connectivity.

The wire codecs implement Ethernet II (60–1514 bytes without preamble/FCS),
Ethernet/IPv4 ARP, IPv4 with a fixed 20-byte header, ICMP echo request/reply, and IPv4 UDP.
Encoders zero Ethernet padding. IPv4 total length bounds the ICMP slice, so
padding never becomes echo data. Parsers check sizes before indexing, validate
addresses and checksums, reject unsupported EtherTypes/protocols/options, and
reject reserved flags, fragments and zero TTL. The interface MTU is explicitly
configured from 68 to 1500 bytes; oversized TX is refused and RX
fragmentation/reassembly is explicitly deferred.

The ARP cache has eight entries and no pending packet queue. `resolve` reports
ready/request/wait; callers poll using monotonic milliseconds. Resolution sends
at most three requests, one second apart, then returns `Timeout` and releases
that entry on the next poll. Reachable entries expire after 30 seconds into
`Stale`; the next use must resolve again. Stale slots can be reused. Pending
entries remain bounded and require polling to finish their retry lifecycle.
Clock regressions are rejected. These constants bound the diagnostic core;
a future service must establish approved interface policy and configuration.

Only unicast replies for an outstanding request in its current retry window
can update the cache. Ethernet and ARP sender MACs and the local target must
match. Unsolicited replies and replacement of reachable entries are refused.
This reduces passive cache poisoning; ARP has no authentication, and an attacker
racing a solicited reply is not prevented by these checks. Requests to the
logical peer are answered without learning a cache entry.

The logical peer answers at most eight ICMP echo requests per one-second window,
never answers an echo reply, and rejects packets addressed to another peer.
It remains an endpoint with no forwarding. The shared diagnostic
checks three complete echo exchanges (ARP miss, hit and expiry), checks returned
addresses/identifiers/sequences/payload bytes, and observes checksum rejection,
ARP retries/timeout and unsolicited reply rejection. Its synthetic clock makes
this deterministic in both host and native executions.

Each `Peer` owns immutable MAC/IP/interface MTU configuration and a bounded
route table: one protected connected route and up to seven removable gateway
routes, including a default route. Netmasks must be contiguous, route networks
canonical, and gateways valid unicast hosts on the connected subnet (excluding
self and network/broadcast addresses; /31 and /32 use host semantics). Duplicate
prefixes are refused. Longest-prefix matching selects the next hop; no matching
route returns `NoRoute` before touching ARP state. Route updates validate before
mutation. No interface selection, metrics, forwarding or service configuration
is implied.

Echo TX resolves the selected next hop through the existing bounded ARP cache
and retains the final destination in IPv4. Echo responses check a reverse route;
direct responses use the ingress sender MAC, while gateway responses resolve
the gateway. An unresolved reverse gateway emits ARP and requires the sender to
retry its original request: there is no pending packet queue. Interface MTU
checks precede TX neighbor changes, and oversized RX is refused. A second shared
diagnostic checks two off-subnet echo exchanges, default removal/no-route and
MTU rejection. Its synthetic gateway ARP address and remote echo address share
a MAC; this checks emitted next-hop and endpoint bytes without claiming router
forwarding or an external NIC exchange. Its counts use a separate native marker
and preserve the original diagnostic contract.

Host verification compiles the actual library twice (production metadata and
its unit tests), without building the OS or creating a Cargo target directory:

```sh
RUSTUP_HOME=/workspace/.rustup CARGO_HOME=/workspace/.cargo \
  RUSTUP_TOOLCHAIN=stable RUSTC=/workspace/.cargo/bin/rustc \
  python3 tools/test_net_core.py
```

The tests also cover full cache refusal, stale-slot reuse, malformed checksums,
lengths, TTL, flags, unknown protocols, size boundaries, padding, rate limits,
a deterministic 10,000-input corpus and byte mutations inside valid framing,
route capacity/reuse, overlapping/default prefixes, invalid masks/gateways,
reverse gateway ARP and configured MTU boundaries.
Native verification uses `tools/net_core_markers.json` with the shared QEMU
harness. Native success proves the SDK app and logical peer executed, not NIC
I/O. Production packet authority, transport ownership/revocation, netd,
production interface/routing policy, ICMP error generation, configuration and
the full N1 NIC acceptance gate remain open.


## UDP and DNS logical endpoints

`Peer` now demultiplexes UDP into eight bound local ports. Port zero allocates
from 49152–65535; duplicate ports and full endpoint tables fail explicitly.
Each port queues at most four datagrams, each at most 1472 bytes (and bounded
by the interface MTU). A full queue drops the incoming datagram and returns
`Capacity`, preserving FIFO contents. Connected ports filter the remote IP/port;
reconnecting discards old queued packets. Close frees the port and its queue.
`udp_ready` observes queued input; empty receive returns `WouldBlock`.
`udp_recv_into` consumes one datagram and reports both copied and original
lengths, including zero-buffer truncation. There are no blocking operations or
implicit receive timers; callers own scheduling and deadlines.

TX returns distinct `Neighbor`, `Waiting` and `Sent` outcomes. ARP frames are
never counted as transmitted datagrams; the caller exchanges ARP and retries.
Route/MTU validation precedes neighbor mutation. UDP checks the pseudoheader,
ports and exact length within IPv4 total length; Ethernet padding is excluded.
IPv4 zero UDP checksums are accepted; TX always emits a checksum, encoding a
computed zero as 0xffff. No fragmentation or implicit pending TX queue is added.

`dns::Resolver` owns one connected ephemeral port and one transaction, supplied
with a caller-configured IPv4 DNS server. It sends A and AAAA questions over the
same UDP path and validates transaction IDs, exact question/name/type/class,
response opcode/flags/rcode, all section framing and record lengths. DNS names
are normalized to lowercase ASCII labels with a 253-byte limit; labels accept
letters, digits, hyphens and underscores. Root names, IDNA conversion, EDNS,
DNSSEC and arbitrary binary labels are outside this resolver's supported subset.
Messages are at most 512 bytes with 32 records; compressed names use backward
pointers, at most 16 hops and bounded expanded lengths. CNAME resolution follows
at most eight aliases and rejects cycles. Only answer-section A/AAAA records
satisfy the question; authority/additional records cannot supply an answer.

Three transmitted queries are allowed, one second apart, followed by `Timeout`
at the final deadline. ARP attempts do not consume DNS attempts. Polling drives
all work; clock regressions fail. Malformed or mismatched replies return an
explicit error while preserving the transaction; NXDOMAIN terminates with
`NameNotFound`. The TC bit returns `Unsupported`: TCP fallback is deferred,
never reported as a successful answer. Positive answers use an eight-entry FIFO
cache; expiry uses the minimum TTL across the CNAME chain. Zero TTL is not
cached, expired entries are removed, and returned cache TTL is remaining whole
seconds. Negative caching is deferred. This bounded diagnostic resolver has
predictable IDs/ports and no DNS authentication; it is not a production resolver.

The shared UDP/DNS diagnostic exchanges nineteen datagrams, checks A/CNAME/AAAA
answers and TTL cache expiry, wrong-ID/compression-loop rejection, three-send
timeout, queue overflow, checksum failure, FIFO/truncated receive, NXDOMAIN and
explicit truncated-response refusal. Separate `[NET-UDP]` and `[NET-DNS]` native
markers supplement the preserved echo/ARP/routed markers. Host checks also cover
all short lengths, zero/negative-zero checksums, endpoint capacity/reuse,
connected filtering, MTU/route refusal, malformed DNS framing, bounded mutation
corpora and monotonic clocks.

This work adds no NIC transport, kernel path, syscall, rights, public socket ABI,
netd, DHCP state machine, configuration precedence or TCP implementation. N2 and
its external-network acceptance remain open; the native diagnostic establishes
logical owned-frame execution only.
