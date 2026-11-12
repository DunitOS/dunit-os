# OP3 — owned dispatch backing and active-root lifetime

12 November 2026. Dispatch no longer keeps Process/thread references across
run_user_context. A DispatchFrame copies CpuContext and FS base, and retains
Arc owners for the selected task's kernel stack, FPU state and its owner's
address space. Preparation ends its process access before the unsafe HAL entry.
The frame contains no lock guard; address-space guards end before user execution.

## Ownership and restoration

Kernel stacks use opaque interior-mutable bytes: immutable ownership can be
shared while the CPU writes stack memory, without exposing an ordinary byte
slice. The aligned FXSAVE buffer keeps its existing UnsafeCell semantics inside
an Arc. The stack top is derived from its allocation; the redundant stored top
field is removed. AddressSpace is shared through Arc<IrqSafeSpinLock<_>>;
map/unmap/protect/translation and ELF loading use scoped guards. These guards
remain borrowed and cannot escape the current-process callback lifetime.

DispatchScope retains the active address-space owner between iterations. When
switching to a new root, it releases the previous owner only after the switch.
Normal owner exit switches away before releasing VM resources. Scope Drop
restores the caller's CR3, PID and selected syscall stack on all return/error
paths. Caller stack/space owners are also retained during the scope. This avoids
an extra kernel-root roundtrip on each yield. Every switch still uses the VMM
path that refreshes the canonical kernel half, including a repeated CR3 value.

After HAL returns, FS/FPU pointers and selected syscall stack policy are reset
before the task frame can be released. The active-root owner remains until it
is safe to retire its page tables. The entry assertion rejects IRQ-masked
userspace dispatch. This is the current UP userspace dispatch path, not a new
nested kernel-thread execution model.

## Native proof and regression checks

The new DISPATCH-PIN smoke constructs a real address space with a mapped private
page, stack and FPU backing, then drops the Process. Weak references and an actual
translation verify that the dispatch frame still owns those allocations. A real
root-switch scope then outlives the task frame: stack/FPU owners are reclaimed,
but the active address space stays alive. An intentional error return must
restore the previous PID/CR3 before dropping the last root owner. The smoke does
not execute synthetic user instructions; the native app scenarios exercise real
HAL execution, FPU/TLS isolation and preemption.

```sh
python3 tools/test_process_resources.py --cpus 4 --accel tcg
```

The strengthened positive and negative budgets require the new ownership proof.
Fifteen positive fd/VFS/IPC/lifecycle scenes and the separate thread fault/join
scene passed, including 1,000 joinable TLS/errno/allocation cycles and 1,000
detached auto-reaps. GUI smoke passed its unchanged 30-marker budget on four
vCPUs. Native C pointer lock/confine/release and exact cursor hide/restore/exit
pixels passed on eight vCPUs. Full boot/DF/IRQ checks and the fixed-work native
CPU/allocator/MemFS fixture passed on one vCPU. Production and boot-smoke kernel
checks, host lifetime/guard/queue/QEMU tests and generated ABI/layout checks pass.

## Coarse before/after measurements

The before image was verified against the committed process-access receipt,
not inferred from the edited worktree at run time. Both series use the same
native TCC fixture, one vCPU, TCG, 512 MiB, one warmup and five measured trials
per scene. All 36 records per series retain identical work sizes/checksums.

| Scene | Before median ms | After median ms |
| --- | ---: | ---: |
| cpu-1 | 50.0 | 40.0 |
| cpu-2 | 60.0 | 50.0 |
| cpu-4 | 50.0 | 50.0 |
| cpu-8 | 60.0 | 50.0 |
| allocator | 280.0 | 240.0 |
| memfs-io | 30.0 | 30.0 |

These PIT elapsed values have 10 ms resolution, IRQ-off undercount and shared
host noise. They establish no speedup, CPU accounting, scheduling-tail bound or
KVM scaling result. [Records and VM receipts](dispatch-ownership.json) retain
the measurements, image hashes, build provenance and successful outcomes.

## Remaining gates

The process registry still uses UP-only mutable access. The resource owners here
pin hardware backing, not the whole task's metadata or Running claim. A scoped
registry/task locking model, lifecycle cancellation and atomic wait/wakeup are
still required. Pinning AddressSpace does not pin individual mappings against
unmap or transfer Process shared-frame bookkeeping into that object; active CPU
tracking and acknowledged TLB shootdown/reclamation remain open. APs continue to
park and applications use the UP scheduler. No public ABI, GUI wire format,
rights model or persistent filesystem format changes.
