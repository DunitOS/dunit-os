# E1000 packet bring-up (N0 diagnostic slice)

The production network path still performs PCI/status/MAC discovery. No socket,
`netd`, raw-NIC capability or application packet queue is provided by this change.
The private boot diagnostic is compiled only with `boot-smoke-tests` and runs
after clock initialization. It uses the QEMU 82540EM reference (8086:100e), not
all devices classified as E1000 by discovery.

Production discovery now enables only PCI MEMORY_SPACE while reading registers.
Its scoped mapping restores the original command bits with BUS_MASTER cleared,
including invalid-register and map-failure paths. It never resumes ambient
firmware DMA without an owning queue driver. Status/MAC/classification markers
remain unchanged.

## Ownership and packet path

Legacy RX and TX rings contain 32 aligned 16-byte descriptors each. Ring lengths
are multiples of 128 bytes. Every descriptor has a private 2 KiB buffer inside
one PMM-contiguous, zeroed allocation. CPU and physical DMA addresses remain
separate. TX reserves one descriptor to avoid a full ring looking empty when
head equals tail. No buffer is rewritten until its descriptor reports DD; TX
completion errors fail instead of becoming successful reclaim. A SeqCst DMA
publication fence precedes MMIO tail writes on the current coherent x86 target.

RX waits for DD, acquires DMA visibility, checks EOP/errors and the 14–1518 byte
frame bound, then copies to the caller's bounded output. Oversize/error frames,
short output and every descriptor in a fragmented chain are dropped and recycled.
CPU ownership ends before RDT publication. This layer performs no ARP, IPv4,
checksum offload or routing policy. Counters distinguish queued/completed TX,
bytes, full-ring refusals, RX packets/bytes and drops.

The QEMU 82540EM model implements loopback through PHY BMCR bit 14, read/written
via bounded MDIC transactions at PHY address 1. RCTL's MAC-loopback bits do not
select its transmit loopback path. The model also suppresses receive for 1000 ms
after every RCTL write (`set_rx_control` / `flush_queue_timer` in QEMU
`hw/net/e1000.c`, verified against v10.0.2; the installed runner is 10.0.13).
The private reference diagnostic therefore waits 1010 ms after final RX setup
before its first send; the per-packet deadline remains 100 ms. Original PHY
control is restored on stop. This is a diagnostic emulator constraint, not a
production packet-path delay or a timing workaround hidden in netd.

The local diagnostic programs PHY BMCR loopback, masks interrupts and sends 103
128-byte frames through actual TX and RX DMA rings. Destination/source are the
NIC MAC; the local experimental EtherType is 0x88b5. Every byte varies with packet
sequence and must match on receipt. Three wraps plus seven descriptors exercise
both ring boundaries. These frames are PHY-local and do not require public
Internet or emit network traffic on the host link.

## Stop and failure

The private DMA owner starts CPU-owned. Publishing a hardware queue arms its
Drop guard: pages cannot be returned to PMM until the controller owner observes
reset completion, PCI bus-master disable and device readback. Stop errors retain
the DMA backing; they do not pretend that RX/TX enable bits alone prove quiescence.
The driver rejects another diagnostic after failure. Cleanup never restores
ambient firmware bus mastering. The native probe compares PMM free bytes before
allocation and after controller stop/drop.

A quarantined allocation is deliberately retained for the rest of the boot;
there is no recovery/reclaim contract yet. Observed QEMU reset is not proof of
all real-controller DMA/posting behavior. Safe hardware recovery and final DMA
retirement remain N0 gates. The allocation owner is NIC-private; a common DMA
API should be introduced when a second real consumer can share its contract,
rather than publishing an unused abstraction.

## Verification

Host command (set RUSTC/RUSTUP environment to the installed host compiler):

```sh
python3 tools/test_net.py
```

It compiles the actual DMA owner, ring and controller modules with RAM-backed
physical/MMIO transport shims. The checks cover descriptor sizes/offsets,
103-packet wrap/payload/completion, one-slot reservation and full-ring refusal,
malformed/fragmented/oversize RX, short output, completion errors, OOM, initial
reset failure, stalled TX, corrupted data, failed stop reset and bus-master-disable
refusal. A separate host check compiles the actual production discovery module
and asserts MEMORY-only command writes, preservation of other command bits,
non-restoration of firmware bus mastering and mapping cleanup on success,
invalid registers and map failure. Uncertain completion must keep every DMA page allocated. The host reference transport now rejects RX without both PHY loopback and
the one-second activation interval; a negative test checks these conditions. Host test
teardown releases quarantined RAM only after destroying the simulated hardware;
production has no such unchecked release.

Native validation uses the shared harness and extends the existing complete
boot regression marker contract without weakening its assertions:

```sh
python3 tools/qemu_test.py --build \
  --markers-file tools/net_n0_markers.json --cpus 4 --accel tcg \
  --qemu-arg=-nic --qemu-arg=user,model=e1000 \
  --timeout 600 --cmd-timeout 240 --boot-timeout 120
```

Required observed result:

```
[NET-N0] e1000 PHY-loopback packets=103 bytes=13184 ring-wrap, payload, reset and DMA reclaim OK
```

`[NET-N0] FAIL` or a missing marker fails the suite. A machine without the
reference NIC reports a skipped diagnostic; the N0 suite requires success and
therefore rejects that skip. Native results must be reported separately from
host model checks; this document alone does not claim a fresh VM run.

## Remaining N0 and M8 work

This is diagnostic polling, explicitly not accepted IRQ-driven N0. MSI/MSI-X and
legacy IRQ delivery/moderation, link transition/stall recovery, RX starvation,
interrupt storms and the ten-minute flood matrix remain open. External RX/TX,
multiple NICs, hardware DMA recovery and stats publication also remain open.
The approved `NET_DEVICE`/`PACKET_QUEUE` authority, bounded shared-ring ownership,
revocation/netd restart and service protocols must precede application exposure.
Ethernet/ARP/IP/ICMP (N1), UDP/DHCP/DNS, TCP, sockets/musl and TLS are not provided.

Fresh TCG native checks and their source/image hashes are recorded in the
[17 November integration receipts](../verification/multitrack-2026-11-17.json).
