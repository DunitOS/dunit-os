# Userspace packet core (N1 foundation)

`dunit-net-core` is a dependency-free `no_std` + `alloc` library shared by the
native SDK `net_packet_test` app and host tests. It has no syscall, kernel,
MMIO, DMA, device handle or service dependency. `Peer` exchanges owned frame
vectors with a logical peer supplied by its caller. It does not open an
interface, send packets to a NIC, or provide Internet connectivity.

The wire codecs implement Ethernet II (60–1514 bytes without preamble/FCS),
Ethernet/IPv4 ARP, IPv4 with a fixed 20-byte header, and ICMP echo request/reply.
Encoders zero Ethernet padding. IPv4 total length bounds the ICMP slice, so
padding never becomes echo data. Parsers check sizes before indexing, validate
addresses and checksums, reject unsupported EtherTypes/protocols/options, and
reject reserved flags, fragments and zero TTL. The MTU is 1500 bytes; oversized
TX is refused and RX fragmentation/reassembly is explicitly deferred.

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
It is an endpoint, with no forwarding or routing table. The shared diagnostic
checks three complete echo exchanges (ARP miss, hit and expiry), checks returned
addresses/identifiers/sequences/payload bytes, and observes checksum rejection,
ARP retries/timeout and unsolicited reply rejection. Its synthetic clock makes
this deterministic in both host and native executions.

Host verification compiles the actual library twice (production metadata and
its unit tests), without building the OS or creating a Cargo target directory:

```sh
RUSTUP_HOME=/workspace/.rustup CARGO_HOME=/workspace/.cargo \
  RUSTUP_TOOLCHAIN=stable RUSTC=/workspace/.cargo/bin/rustc \
  python3 tools/test_net_core.py
```

The tests also cover full cache refusal, stale-slot reuse, malformed checksums,
lengths, TTL, flags, unknown protocols, size boundaries, padding, rate limits,
a deterministic 10,000-input corpus and byte mutations inside valid framing.
Native verification uses `tools/net_core_markers.json` with the shared QEMU
harness. Native success proves the SDK app and logical peer executed, not NIC
I/O. Production packet authority, transport ownership/revocation, netd,
connected/default routes, ICMP error generation, configuration and the full
N1 NIC acceptance gate remain open.
