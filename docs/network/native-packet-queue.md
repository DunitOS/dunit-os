# Native Ethernet queue and boot service

The maintainer approved the copied-batch design in the
[packet authority decision](packet-authority-proposal.md). The first device is
the QEMU 82540EM E1000 on legacy INTx. `NetDeviceGrant` (95) publishes a
nontransferable device handle to a live direct child of the boot supervisor.
`NetQueueOpen` (96) opens a process-local native descriptor. Device control and
queue access use bits `NET_CONTROL` and `NET_RAW`, with `READ` and `WRITE`
attenuating operations. See the [ABI description](../../abi/README.md#native-ethernet-packet-queue).

GUI boot starts the supervisor from the boot archive and supplies the
`DUNIT_NET_ROOT` handle number. The supervisor accepts grants only from
`/system/boot-services`, using executables from `/system/libexec`. Both
namespaces are sealed after archive population. Regular app launches and
mutable service manifests cannot obtain this bootstrap permission. Init
delivers the granted handle number in sender-attested IPC and retires the
child if delivery fails. The default manifest grants read and write to netd.

Queues share one NIC generation. Same-process dup shares the generation,
and the final descriptor close or final device-handle close ends it. Process
teardown also closes live queues. Revocation wakes readiness waiters and
ignores stale generation tokens. Spawn excludes the queue even when the
descriptor's close-on-exec flag was cleared. Kernel DMA and device registers
stay private.

Write validates the entire copied batch before publishing a prefix. Read
reserves a CPU-owned snapshot and consumes frames after copying into checked
user pages. Mapping faults preserve the reserved frames. Legacy IRQ work
reclaims TX and drains bounded RX without allocating. Stop masks causes,
excludes the IRQ handler, verifies reset and clears PCI bus mastering before
releasing DMA. An unverified stop quarantines the NIC.

The reference netd accepts an optional address and prefix such as
`192.0.2.2/24` in the boot archive at `/system/boot-services/netd-ipv4.conf`.
It responds to ARP and ICMP for that address. The default image opens the queue
and leaves IPv4 unconfigured.

Run the isolated wire and restart test with
```sh
python3 tools/qemu_test.py --build --markers-file tools/net_live_markers.json \
  --cpus 8 --accel tcg --timeout 420 --boot-timeout 180
```
The 18 November run checked two ARP/ICMP/UDP DNS exchanges with an isolated
host peer, process restart, stale queue revocation and desktop startup.
The unmodified DWM profile booted the reference service separately.
See the [verification receipt](../verification/native-packet-2026-11-18.json).

Application socket/RPC interfaces, DHCP, production TCP/DNS integration,
MSI/MSI-X, multi-NIC support and hardware recovery/flood qualification remain open.
