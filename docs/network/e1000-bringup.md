# E1000 packet bring-up (N0 diagnostic slice)

This document records the private PHY loopback and external INTx diagnostics.
The [native packet queue](native-packet-queue.md) describes the production
path built on the same reference controller. The private boot diagnostic is
compiled only with `boot-smoke-tests` and runs after clock initialization. It
uses the QEMU 82540EM reference (8086:100e).

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

The PHY-local diagnostic remains polling. A separate opt-in private external
INTx probe is described below; neither diagnostic closes full N0. The 82540EM
reference uses legacy INTx and does not prove MSI/MSI-X. Link transition/stall
recovery, RX starvation, interrupt storms, moderation and the ten-minute flood
matrix remain open. Multiple NICs, hardware DMA recovery and stats publication
also remain open.
The native packet queue uses copied batches and kernel-owned DMA. Application
socket protocols, DHCP, TCP and TLS remain open.

Fresh TCG native checks and their source/image hashes are recorded in the
[17 November integration receipts](../verification/multitrack-2026-11-17.json).


## Private external INTx slice

`boot-smoke-tests` plus the exact Limine token `net_irq_test=1` opts into an
external 82540EM probe after the existing PHY test. An ordinary boot runs no
external exchange. The reference is PCI 8086:100e on legacy line 10 or 11; this
controller does not provide MSI/MSI-X. This diagnostic runs privately before
any service takes packet-queue ownership.

One static `IrqSafeSpinLock` owns the controller and an inline eight-frame,
128-byte RX queue. Lock acquisition disables interrupts before waiting, and no
guard crosses HLT. The handler reads/clears ICR, masks device causes, reclaims
at most 32 TX descriptors and drains at most 32 RX descriptors. It rearms IMS
and uses ICS if RX work remains, so a bounded drain does not depend on a new
wire edge. Only this handler reads DD and advances RX/TX completion counters.
Submission reads CPU-owned free-slot state; the runner reads the CPU RX queue
and counters. An empty shared-line ICR is not claimed. PIC EOI belongs to HAL
and follows device acknowledgment. Overrun, malformed RX, completion errors or
a full CPU queue fail the private probe and leave its device causes masked.

The scoped route sets the assigned PIC/ELCR line to level triggered and saves
and restores its mask, cascade mask and ELCR bit. PCI interrupt disable is
cleared only during the mapped owner lifetime. The existing reset, observed
bus-master disable, readback and DMA quarantine rules also cover external
start, stop and failure. A probe cannot be retried during the same boot.

The runner waits with HLT and monotonic deadlines, including QEMU's receive
activation interval. It exchanges 103 frames of 128 bytes, spanning three ring
wraps plus seven descriptors. The Ethernet destination is 02:00:00:00:00:01,
source is 52:54:00:12:34:56 and EtherType is 0x88b5. Request payload byte `i`
is `(sequence * 37 + i) & 255`, with `i=0` at Ethernet offset 14. The host swaps
MAC addresses and XORs every payload byte with 0xa5. The guest rejects an
untransformed local loopback frame, a wrong sequence or any corrupt byte.

`tools/net_test_transport.py` listens only on 127.0.0.1, using QEMU's four-byte
big-endian length framing, and checks every byte before replying. Its shutdown
is bounded even when QEMU never connects or sends an incomplete frame. The
harness also requires its `net-peer.json` receipt to show exactly 103 checked
requests and 103 responses; guest serial success alone is insufficient.

Host checks:

```sh
RUSTUP_HOME=/workspace/.rustup CARGO_HOME=/workspace/.cargo \
  RUSTUP_TOOLCHAIN=stable RUSTC=/workspace/.cargo/bin/rustc python3 tools/test_net.py
python3 tools/test_net_transport.py
```

The actual controller host checks include IRQ-exclusive progress, shared-line
acknowledgment, three wraps, full TX without handler reclamation, bounded CPU
queue exhaustion, untransformed loopback rejection and payload corruption.
They use simulated hardware and do not prove native IRQ routing.

Native validation (the harness supplies the isolated socket peer and exact
boot token when building):

```sh
python3 tools/qemu_test.py --build --net-test \
  --markers-file tools/net_irq_markers.json --cpus 4 --accel tcg \
  --timeout 600 --cmd-timeout 240 --boot-timeout 120
```

Reusing this ISO also requires `--net-test`; the embedded token still requests
the peer exchange. The marker contract retains the complete existing boot
regression and PHY marker, adds the external IRQ success marker and forbids
`[NET-IRQ] FAIL`. A fresh native run and image/source receipt must be recorded
separately. [Follow-up receipts](../verification/multitrack-progress-2026-11-17.json)
record passing 2/4/8-vCPU TCG runs with exactly 103 checked host requests and
responses each, transformed-payload validation and guest IRQ/reset/reclaim
markers. This accepts the private diagnostic, not the full N0 production gate.
