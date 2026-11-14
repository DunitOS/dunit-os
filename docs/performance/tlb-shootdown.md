# Acknowledged TLB invalidation and user mapping ownership

14 November 2026. OP3 memory work; this does not enable AP application dispatch.

## Invalidation and reclaim

Each CPU publishes its root before loading CR3. PTE mutation precedes a
sequentially consistent active-root snapshot. User invalidation targets CPUs
using that root; kernel alias invalidation targets every online CPU. A serialized
sender publishes a generation in each recipient's cache-line-separated mailbox,
delivers an NMI and waits for that exact acknowledgement. Mailbox publication is
fenced before APIC delivery, including the x2APIC WRMSR path.

The receiver runs on the existing private NMI IST stack, after kernel GS and SIMD
state have been established. It only reads CR3, executes INVLPG where necessary
and acknowledges. It takes no task, mapping, MMIO, heap or sender lock. Thus a
CPU waiting for one of those locks with ordinary interrupts disabled can still
complete the sender's request. A rejected delivery or missing acknowledgement
panics before the caller can reuse the memory. This is fail-closed handling,
not recovery from a failed CPU.

Unmap acknowledges before freeing an owned user frame; protection changes
acknowledge before returning; kernel alias removal acknowledges before its virtual
extent can be reused. Fresh mappings do not broadcast: x86 does not cache
not-present translations, and replacement first requires acknowledged unmap.
User GLOBAL mappings are rejected so CR3 switching preserves this invariant.
An address-space destructor asserts that no CPU still uses its root.

MMIO extent accounting and canonical kernel-half publication use one real
IRQ-safe mutex. The shared kernel MMIO hierarchy is installed before user roots
are created. Temporary firmware aliases retain the physical memory's cache type.
Page-table walks use shared references with atomic 64-bit entries rather than
whole-table mutable references. This also permits hardware accessed/dirty updates
and canonical kernel-half publication beside a separately locked user-half walk.
PTE/table layout is asserted at compile time; mapping transactions still require
their AS or MMIO lock. Atomics do not replace transaction ownership or shootdown.

## Mapping lifetime during dispatch and copies

Dispatch enters an address space while holding its mapping mutex. The CPU-bound
dispatch scope owns the active Arc and publishes its pointer until the actual
root restoration; the old owner is released only after switching away. A user
copy can clone that pin without looking up or relocking the current task.

User-memory access now uses a non-escaping, CPU-bound view. The address-space
mutex spans page validation and every byte or framebuffer row copied. Only
owned bytes/scalars leave the callback. A concurrent unmap therefore cannot
win between pointer validation and dereference. Framebuffer rows retain forward
volatile word-copy semantics, including unaligned source fallback. Thread return
slot initialization precedes Ready publication. Production access without an
active dispatched mapping returns EFAULT; only boot smoke tests retain their
explicit permanent-root bootstrap path.

## Verification and limits

`tools/test_tlb.py` compiles the actual mailbox protocol and SpinLock with host
CPU/IPI/INVLPG transport shims: eight senders perform 8,000 transactions and
56,007 exact acknowledgements, including a delayed response. A dropped response
triggers the real timeout path and prevents the simulated reclaim callback.

`tools/test_user_memory.py` compiles the actual access module: an unmapper waits
for the copy scope; permissions, empty copies, owned snapshots, aligned/unaligned
pixel reads and production no-context refusal are checked. Compile-negative
tests reject an escaped mapping view and migration to another host thread.

The native probe uses actual AP CR3/PTE/TLB accesses with ordinary IRQs disabled.
Across 32 rounds, BSP downgrades permissions, removes a mapping and retargets it
between two separately owned physical pages. A kernel alias is also removed and
reused. AP reads must observe the expected contents through both addresses.
Alternating rounds keep APs entered or require an explicit left-generation
acknowledgement before mutation, then reenter under the mapping mutex. Final
PMM accounting verifies reclaim of the two pages and all owned page-table frames.
The dispatch probe also copies through the active pin after Process/Frame drop,
inside an already-held task scope, and checks final root/pointer restoration.

```sh
python3 tools/test_tlb.py
python3 tools/test_user_memory.py
python3 tools/test_process_resources.py --cpus 4 --accel tcg
python3 tools/test_tlb_native.py --reuse-image --cpus 1 4 8 --accel tcg
```

[Structured receipts](tlb-shootdown.json) distinguish image/source versions.
These probes do not prove remote CPL3 W^X fault handling, DMA quiescence,
concurrent process cancellation, all driver lock orders or parallel applications.
Application scheduling remains UP; KVM scaling requires another runner.
