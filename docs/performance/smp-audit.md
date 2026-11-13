# SMP shared-state audit — OP1

Initial audit: 11 November 2026. Updated on 13 November 2026; implementation receipts below.
Audit of the UP implementation; this inventory is a safety
gate, not a claim that shared state is now safe to execute on APs.

| State/path | Existing protection/assumption | Required ownership and gate |
| --- | --- | --- |
| HAL syscall scratch, selected/escape stack, user FPU pointer | CPU-local GS anchor and private entry state; native syscall/IRQ/DF checks | preserve entry invariants when enabling AP dispatch; no shared writable entry scratch |
| GDT/TSS/IST and initial syscall stack | private per-CPU descriptors/stacks; shared published IDT | retain CPU-private ownership and publication during scheduler activation |
| current PID, exit/yield/schedule/escape flags | CPU-local runtime fields; aggregate counters/terminal ownership remain shared | implement atomic task execution ownership and remote scheduling |
| PROCESS_TABLE and current process/thread access | external access uses non-escaping scopes; fd snapshots are owned; private registry/selectors still UP-only; dispatch backing is independently owned | registry/task pinning and exclusion; remove remaining internal aliases and preserve owned dispatch lifetime |
| Scheduler ready Vec and process state | queue-only IRQ-safe lock; state checks outside lock; generation-tagged candidates | atomic Ready→Running(cpu) claim and process/scheduler lifecycle ownership remain open |
| Wait queue, futex, pipe/PTY and IPC wakeup | UP check/block transaction; generation-bounded wake selection; callbacks outside queue lock; futex task lookup and mapping lock precede queue lock; atomic word compare/register under queue | atomic predicate/block/wake protocol, remote runqueue notification, no waiting under IRQ spinlocks |
| PMM bitmap, heap and shared VM objects | separate locks/atomics, UP callers | cross-CPU allocation/lifetime audit and lock order; reclaim only after final reader |
| Page tables, CR3 and MMIO mappings | Arc-owned locked AddressSpace; dispatch scope pins active root; local invalidation | address-space active CPU mask, generation/ack TLB shootdown before reuse; prevent new entrants racing invalidation |
| VFS registry and filesystem backends | scoped IRQ-safe registry guard; Arc-owned backend locks shared by mounts/files/capabilities | coarse registry scope; complete process/task and ProcFs snapshot lock order before AP dispatch |
| VFS path scratch | previously one shared UnsafeCell buffer | per-operation stack buffers; rename keeps both paths alive without heap copies |
| DunitFS/block drivers | owned/locked DunitFS backend; shared controller/device state | requests serialized by device, completion paths cannot require a lock held by waiter; durability tests unchanged |
| Device IRQ, keyboard/pointer queues and display | INPUT_MASTER/DISPLAY_MASTER and IRQ-safe locks | one routed device IRQ owner initially; cross-CPU locks still needed for syscall readers; no desktop policy in kernel |
| serial/framebuffer terminal and foreground state | shared sink and formatting/state | bounded critical sections, no per-event hot-path trace; console ownership is global policy, not CPU-local |
| CPU clock/deadlines | PIT BSP ticks and SFMASK IRQ-off syscall entry | distinguish clocksource from per-CPU event timers; no cross-CPU TSC subtraction without synchronization checks |

Sequence: first validate CPU-local entry on one CPU with the full boot/DF/IRQ,
TLS/FPU and syscall contracts. Then APs may initialize private hardware state and
park, handling only isolated APIC probes. They must not access process/VFS/heap
paths until their ownership and remote invalidation invariants are implemented.
An idle AP count is not parallel userspace or a scaling result.

CPU-local entry/flags and private descriptors/stacks are now implemented and
checked by the [bring-up receipt](smp-bringup.md). The process ownership and process/VFS lock-order,
wait/wakeup and remote TLB gates in this table remain unresolved; AP application
dispatch stays disabled.

## Guard type guarantees (12 November 2026)

`InterruptGuard` is CPU-bound (`!Send`/`!Sync`): restoring its saved IF on a
second CPU would corrupt both CPUs' interrupt state. Lock guards are `!Send`
so critical sections cannot migrate implicitly. A shared `SpinLockGuard<T>`
requires `T: Sync`; deriving that property from `SpinLock<T>: Sync` alone would
incorrectly allow sharing `Cell<T>` through a guard. IRQ-safe lock guards inherit
the CPU-bound IRQ guard. Zero-sized markers enforce these rules at compile time.

`tools/test_sync_guards.py` compiles the actual kernel module: invalid Send and
Sync bounds must fail, while a shared guard for u64 must compile. Its host-safe
runtime test uses eight threads for 80,000 invariant-checked updates, failed and
successful try_lock operations, and OnceCell publication. Privileged IRQ paths
are exercised by native boot/DF/IRQ and CPU/pthread, allocation and MemFS tests
through `test_smp_boot.py --cpus 1 4` on TCG. These checks do not resolve the
process/VFS lifetime, lock-order or TLB gates above.

The [OP3 VFS ownership receipt](vfs-ownership.md) records removal of raw backend
pointers and global mutable VFS references, pinned lifetime/close checks and
native file/capability, persistence and GUI regressions.

The [process resource receipt](process-resources.md) records detached fd/VM/handle
cleanup, transactional fd inheritance and owned fd snapshots. Resource release
was moved out of lifecycle mutations; process-table/current-task pinning and
scheduler/wait ownership remain open before AP dispatch.

The [scheduler queue receipt](scheduler-queue.md) records removal of the queue ->
process-table lock edge, generation-safe unlocked validation, deterministic
reentry/ABA tests, native regressions and a coarse fixed-work before/after run.

The [current-process access receipt](process-access.md) records privatized legacy
selectors, scoped reads/mutations across syscall/input/IPC paths and deferred fd
wait-set notifications. It enforces a result-lifetime boundary, not SMP exclusion.

The [dispatch ownership receipt](dispatch-ownership.md) records independent
stack/FPU/address-space owners, copied HAL input and caller restoration on error.
No Process reference or address-space guard spans userspace execution; registry
ownership and cross-CPU mapping/reclamation gates remain open.

The [wait queue receipt](wait-queue.md) records callbacks outside registration
storage locks, generation-bounded FIFO selection and scheduler admission after
the mutable process-table borrow. The subsequent [futex admission receipt](futex-admission.md)
moves task lookup and AddressSpace acquisition before queue locking, keeps the
word view borrowed from the mapping guard and uses Acquire/Release word access.
This does not provide registry/task exclusion or the cross-CPU block/wake/cancel
protocol; task state transitions still rely on the outer UP IRQ boundary.
