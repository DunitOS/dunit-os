# Physical allocator exclusion and AP proof — OP3

13 November 2026. PMM operations are now protected independently of the remaining
process registry/task work. AP application dispatch stays disabled.

## Confirmed defect and change

`PhysicalMemoryManager` previously held its bitmap in UnsafeCell, asserted Sync
manually and mutated bits through `&self` without exclusion. Atomic free/usable
counters did not make scan/reserve/clear atomic. Two CPUs could select the same
zero bit or race a contiguous reservation against individual allocations.

The mutable slice and both counters now belong to one FramePool behind an
IrqSafeSpinLock. Scan + reservation + accounting, contiguous scan + whole-run
reservation, and clear + accounting each run under that lock. Stats take one
consistent snapshot. There is no manual manager Sync implementation or runtime
bitmap UnsafeCell; the type derives its sharing guarantees from the real lock.
Single and contiguous calls retain first-fit behavior. Tail bits are reserved,
constructor capacity/alignment/overflow are checked, and invalid unaligned frees
cannot accidentally clear a neighboring frame.

A separate initialization lock covers both the initialization decision and
taking the one static mutable bitmap slice. Repeated/concurrent init returns the
published manager without resetting live allocations. The boot backing storage
still has the narrowly scoped UnsafeCell required for this single handoff.

Lock order: heap / AddressSpace / device owner → PMM pool. PMM is a leaf: no heap
allocation, logging, mapping, process callbacks, remote acknowledgements or waits
while its pool lock is held. Bulk contiguous scan is still a global IRQ-off
critical section; contention/scanning optimization needs profiling separately.

## Actual BSP/AP test

The boot-smoke kernel runs a one-shot rendezvous after AP hardware initialization.
IPI vector 244 enters a dedicated AP probe; every AP acknowledges entry before
the BSP releases the start gate. Delivery/entry failure aborts the gate before
any Rust barrier/allocation job starts. Duplicate probe interrupts cannot rerun
a slot. Production kernels have no Rust probe worker; the HAL weak reference
remains inactive and no request is issued.

Each CPU performs 128 rounds, alternating one four-frame contiguous allocation
and four individual allocations. All CPUs publish their frame identities, then
rendezvous so every cohort is simultaneously live. BSP checks every pair for
duplicates before any participant frees. A second barrier protects the check;
a third follows freeing. Every CPU must finish all rounds, HAL must acknowledge
completion, and PMM stats must exactly return to their starting values.

The Rust barriers are bounded and release peers on failure. No PMM guard spans
any barrier/HAL wait. The AP job has no heap/VFS/process entry, page-table edits
or memory mappings. It executes in the isolated probe IRQ and returns to HLT;
this is a boot correctness fixture, not a production interrupt latency policy.

## Reproduction and evidence

```sh
python3 tools/test_pmm_concurrency.py
python3 tools/test_pmm_smp.py --cpus 1 4 8 --accel tcg
python3 tools/test_process_resources.py --reuse-image --cpus 4 --accel tcg
python3 tools/qemu_test.py --build --markers-file tools/pmm_production_markers.json --cpus 4 --accel tcg
```

The host test compiles the actual PMM and SpinLock/OnceCell with only IRQ and
boot-map shims. It checks exhaustion, tail bits, whole-run reservation, malformed
frees, double-free accounting, usable-region holes/partial edges and concurrent
repeat initialization without resetting outstanding frames. Eight host workers
perform 80,000 mixed single/contiguous transactions, verifying each live owner,
consistent counters and final restoration: 239,984 frames allocated and freed.
Existing independent property-test models are retained, but do not stand in for
this actual-module test. The same ownership-only loop was also compiled against
the actual PMM from `67940c6` and this implementation, using the unchanged public
allocation/free/total/available APIs. The old module exited 101 with
`duplicate live physical frame`; the new module completed all 80,000 transactions
and 239,984 frame handbacks. This comparison isolates concurrent ownership; the
full host test above separately covers boot publication and bounds.

Native 1/4/8 CPU runs require every CPU's 128-round proof and the final summary:
512 / 2,048 / 4,096 frame allocations respectively. The unchanged APIC/IPI/timer,
boot/DF and native C fixed-work checksum budgets also pass. The callback checks
its actual hardware CPU index, so the test cannot be satisfied by invoking all
workers sequentially on BSP. Results and final regressions are recorded in
[the receipt](pmm-concurrency.json). Test wall times/PIT samples are not scaling
or syscall latency evidence. The 15 positive resource scenes and the separate
thread lifecycle fixture passed on the same smoke ISO at 4 vCPUs, including
1,000 join and 1,000 detached TLS/errno/allocation cycles. GUI smoke passed all
30 required markers on 4 vCPUs. The terminal ISO adapter follows the unchanged
marker/action reconstruction in [the wait queue receipt](wait-queue.md).
Production and boot-smoke compile, guard/PMM host, ABI/layout, QEMU unit and kernel
ELF checks pass. A separate production-config build/boot checks
that the boot-only Rust callback is absent/inactive while the same native C
fixed-work checksum fixture still passes; `[PMM-SMP]` is forbidden in that run.

## Remaining gates

The pool lock protects reservation/accounting, not whether a caller is allowed
to return a frame. free_frame still requires ownership and retirement of all
readers, mappings, DMA users and TLB entries. Stale/double returns after reuse,
shared-frame bookkeeping, active CPU masks and acknowledged shootdown remain
separate obligations. This does not make all heap/VMM/device paths SMP-safe.
Registry/task ownership, atomic Running(cpu), cross-CPU wait/cancel, remote wakeup
and parallel application execution remain open. KVM/physical performance is not
measured; APs park after the isolated physical allocator test.
