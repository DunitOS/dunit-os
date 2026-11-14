# Wait admission, shared clock and IPC ownership

14 November 2026. OP0/OP2/OP3 work; applications remain UP. This extends
[execution ownership](task-execution.md) without enabling AP userspace dispatch.

## Wait admission

IPC wait, pipe read/write, descriptor poll and wait-set snapshot the wait queue's
notification checkpoint **before** checking their producer predicates. Admission
compares it while holding the task/metadata/registration locks. Every producer
publishes a new notification generation under that same registration lock, even
when no waiter exists. If a producer wins the check/register gap, the syscall
captures its existing restart/return context, publishes Ready and escapes without
parking. The existing execution lease delays admission until HAL return completes.
An already expired deadline also avoids parking. Sleep has no producer predicate;
futex still compares its atomic user word and registers under the same queue lock.

Registration generations now travel with wake callbacks and are checked against
the task's current wait generation. A delayed callback selected before cancellation
cannot wake a later registration of the same PID. Cancellation detaches queue
registration and pending wait-set under the same task -> metadata -> queue order;
resource destruction remains outside those guards. Notifications are conservative
and global: an unrelated producer may cause a bounded syscall retry. This uses
constant counter storage and does not claim an optimal notification topology.

`tools/test_parking.py` compiles the actual parking, registry, queue and SpinLock
code with privileged capture/IRQ shims. A remote producer wins the gap in all
8,000 forced trials. Tests also delay an old callback across a new registration,
exercise pipe/IPC/poll/wait-set retries and expired deadlines. The native PARKING
proof uses real Process, queue and wake paths; real blocking apps exercise HAL.
Full remote cancellation and process-group stopping remain separate work.

## Clock source

Validated ACPI RSDP/RSDT/XSDT/HPET tables select a 64-bit HPET main counter.
Checksums, bounded table sizes, memory GAS, register alignment, counter width,
period and actual progress are checked. Firmware-table mappings preserve existing
HHDM cache attributes (normal RAM if no alias); HPET registers use device caching.
Temporary firmware mappings are retired. The permanent counter mapping is published
once before AP startup. Missing/invalid/32-bit/stuck HPET falls back to PIT, with
no guessed physical device address and no unvalidated cross-CPU TSC arithmetic.

HPET conversion uses wide arithmetic and saturates at u64 nanoseconds. Monotonic
time continues across IRQ-off sections; coarse scheduler ticks derive from that
source. Deadline duration is represented in ns and rounded upward when converted
to the existing scheduling-tick domain. PIT remains the 100 Hz event source;
higher counter resolution alone does not make wakeups, scheduling or scanout 10 ns.
Native proof checks at least 2 ms of progress with interrupts masked while the
PIT delivery counter stays unchanged. Actual AP IPI reads are bracketed by BSP
counter reads before request/after acknowledgement. Hardware, source resolution
and event source are recorded separately in performance metadata.

The QEMU HPET advertises a 10 ns counter period. That is device resolution,
**not** measurement accuracy or latency. The `hpet=off` machine configuration
must select PIT at 10 ms resolution and pass the original core/hardware budgets.
An initial `-no-hpet` attempt was rejected by this QEMU version before boot;
the supported `-machine hpet=off` property is used for the actual fallback proof.

## IPC ownership and capability publication

A real IRQ-safe IPC guard replaces the global UnsafeCell/static mutable manager.
No manager borrow escapes that guard. Queues use bounded VecDeque storage instead
of moving remaining Message values on every FIFO receive. Byte-send admission
holds a live target task scope; the IPC lock orders process membership validation
with retirement -> mailbox cleanup. Readiness notifications occur after task and
IPC guards end, so callbacks can query IPC again.

Capability insertion and queue publication now share the target task transaction;
rollback runs after the IPC guard ends. Attached receive snapshots carry a unique
packet sequence. A delayed consume must match that exact head before popping it,
so a second receiver cannot remove the next packet by committing an old snapshot.
Queue copy/commit has no IPC guard across user-memory access. Mapping pinning and
concurrent unmap still require the separate VM/TLB work; this is not that proof.
No syscall, GUI wire, rights or filesystem-format change is introduced.

The actual manager/SpinLock host suite checks 8 producers, 8,000 ordered packets,
full-queue limits, short-buffer/attachment error preservation, stale consume,
reentrant notification and compile rejection of a static escaped manager borrow.
The native IPC-LOCK proof checks bounded FIFO, attachment identity and retirement
cleanup with real tasks. Existing GUI and capability regression budgets remain.

## Native IPC baseline and receipts

`assets/tcc/performance_ipc.c` is compiled by TCC inside Dunit. Two independently
spawned processes exchange authenticated fixed-size packets. Warmup plus five
measured trials each verify 1,000 round trips, 64,000 bytes, sender identity,
sequence/payload/checksum and final child exit. JSON records use a single write;
`tools/ipc_performance_report.py` validates every work count and checksum before
reporting a median. This is UP round-trip work, not SMP scaling or input latency.

```sh
python3 tools/test_parking.py
python3 tools/test_ipc_lock.py
python3 tools/test_clock.py
python3 tools/test_process_resources.py --cpus 4 --accel tcg
python3 tools/test_clock_native.py --reuse-image --cpus 1 4 8 --accel tcg
python3 tools/ipc_performance_report.py build/performance/progress-final-regression/qemu-serial.log
```

[Structured results](progress-foundation.json) bind source, images, toolchain,
host, actual clock, work checks and native runs. Coarse prior PIT measurements
must not be treated as comparable speed measurements after changing clocksource.
KVM/reference scaling, IRQ-off profiling, interruptible syscall boundaries,
remote stop/cancellation, TLB shootdown and actual SMP dispatch remain open.
