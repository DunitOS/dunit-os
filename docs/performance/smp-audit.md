# SMP shared-state audit — OP1

11 November 2026. Audit of the UP implementation; this inventory is a safety
gate, not a claim that shared state is now safe to execute on APs.

| State/path | Existing protection/assumption | Required ownership and gate |
| --- | --- | --- |
| HAL syscall scratch, selected/escape stack, user FPU pointer | one global writable block in syscall.asm | CPU-local GS anchor and private entry state; symmetric user transitions and paranoid interrupt recovery |
| GDT/TSS/IST and initial syscall stack | shared BSP objects | private descriptors/stacks per CPU; immutable IDT may be shared after publication |
| current PID, exit/yield/schedule/escape flags | global atomics describe one active task | CPU-local; aggregate counters and terminal ownership remain shared |
| PROCESS_TABLE and returned static mutable Process/fd references | UP IRQ-off sections and stable Box allocation | scoped/owned access, lifecycle pins and exclusion; Box alone does not resolve Rust aliasing |
| Scheduler ready Vec and process state | IRQ-safe global lock; nested process lookup | one explicit lock order; claim Ready→Running(cpu) before dropping scheduler ownership; no duplicate TID dispatch |
| Wait queue, futex, pipe/PTY and IPC wakeup | UP check/block transaction; several independent locks | atomic predicate/block/wake protocol, remote runqueue notification, no waiting under IRQ spinlocks |
| PMM bitmap, heap and shared VM objects | separate locks/atomics, UP callers | cross-CPU allocation/lifetime audit and lock order; reclaim only after final reader |
| Page tables, CR3 and MMIO mappings | local invalidation and global roots | address-space active CPU mask, generation/ack TLB shootdown before reuse; prevent new entrants racing invalidation |
| VFS_INSTANCE, ROOT_MEMFS and VFS_PATH_BUFFER | UnsafeCell and static mutable references | scoped serialization plus lifetime rules; path scratch must be per-call; shared fd offsets need consistent ownership |
| DunitFS/block drivers | shared controller/device state | requests serialized by device, completion paths cannot require a lock held by waiter; durability tests unchanged |
| Device IRQ, keyboard/pointer queues and display | INPUT_MASTER/DISPLAY_MASTER and IRQ-safe locks | one routed device IRQ owner initially; cross-CPU locks still needed for syscall readers; no desktop policy in kernel |
| serial/framebuffer terminal and foreground state | shared sink and formatting/state | bounded critical sections, no per-event hot-path trace; console ownership is global policy, not CPU-local |
| CPU clock/deadlines | PIT BSP ticks and SFMASK IRQ-off syscall entry | distinguish clocksource from per-CPU event timers; no cross-CPU TSC subtraction without synchronization checks |

Sequence: first validate CPU-local entry on one CPU with the full boot/DF/IRQ,
TLS/FPU and syscall contracts. Then APs may initialize private hardware state and
park, handling only isolated APIC probes. They must not access process/VFS/heap
paths until their ownership and remote invalidation invariants are implemented.
An idle AP count is not parallel userspace or a scaling result.

CPU-local entry/flags and private descriptors/stacks are now implemented and
checked by the [bring-up receipt](smp-bringup.md). The process/VFS ownership,
wait/wakeup and remote TLB gates in this table remain unresolved; AP application
dispatch stays disabled.
