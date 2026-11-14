# Registry/task ownership and exclusion — OP3

13 November 2026. Actual locks and owned task leases replace the remaining
private UP-only process table access. The scheduler still dispatches apps on BSP.

## Implementation and ownership

Previously PROCESS_TABLE used UnsafeCell<Option<Vec<ProcessRecord>>> with a manual
Sync implementation; its private getter returned a static mutable Vec. Box kept
Process addresses stable but provided neither exclusion nor lifetime ownership.
The non-escaping public scopes prevented returning borrows but did not fix those
internal properties.

`registry.rs` now stores metadata in IrqSafeSpinLock<Vec<ProcessRecord>>. A record
owns an Arc<IrqSafeSpinLock<Process>> independently of Vec storage/membership.
The guard bounds all metadata borrows; there is no static mutable Vec getter or
manual registry Sync assertion. Lookup clones an Arc and releases metadata before
waiting for the task mutex. Once the task lock is acquired, membership and Arc
identity are revalidated; a copied old lease cannot mutate a replacement PID.
An already admitted callback may complete on its pinned retired object, without
retargeting a replacement. This is lifetime safety, not cancellation semantics.

Current owner/thread and explicit-PID callbacks share this path and cannot return
Process/field borrows. Deferred fd notifications still flush on callback errors,
now after the real task guard drops. Source/target signal scopes remain separate.
Pending wait-set leases are detached and dropped outside the task mutex too.

State/context publication is extracted into `transitions.rs`. Ready/yield/block,
wake, exit and timer preemption use the task mutex and short metadata transactions;
scheduler admission/notifications happen afterward. Runnable observations copy
owners under metadata, inspect task and owner separately (no two task locks),
then revalidate membership/state. They remain observations, not execution claims.
TLS, join/reap/kill and object/resource cleanup use owned detached tasks/batches;
no task mutex wait or resource destructor runs under the registry guard.

Dispatch frames now retain the whole task Arc in addition to copied registers and
stack/FPU/AddressSpace pins. No mutex or metadata guard crosses HAL/user execution.
This keeps task data alive if membership is removed before the frame is retired.
The active-root restoration and canonical kernel-half refresh remain unchanged.
Frame construction borrows its input AddressSpace owner, so an error cannot drop
the last address-space reference inside the metadata transaction.

## Lock order

| Path | Required order/boundary |
| --- | --- |
| Lookup | registry → clone Arc → unlock registry → task |
| Scoped callback | task → short registry identity check → unlock registry → callback |
| State/context commit | task → registry; callback is internal scalar/context/extraction work |
| Futex admission | task → AddressSpace → registry → wait registrations; compare/register stays together |
| Task/VFS/ProcFs | registry released before I/O; ProcFs snapshots acquire metadata only |
| Wake/scheduler | extract registration/state commit, release queue/task/registry before admission/rescan |
| Retire | move record/task/resource batch out, release guards, then run destructors |

Registry code never acquires task, AddressSpace or VFS locks under its guard.
Wait registration commit and heap/PMM storage operations are the explicit leaf
exceptions. A complete device/IPC/VM/IRQ ranking and progress model still requires
separate validation; this table does not claim whole-kernel deadlock freedom.

## Validation

```sh
python3 tools/test_process_access.py
python3 tools/test_user_word.py
python3 tools/test_process_resources.py --accel tcg --cpus 4
python3 tools/test_process_registry.py --reuse-image --cpus 1 4 8 --accel tcg
python3 tools/qemu_test.py --build --markers-file tools/registry_production_markers.json --cpus 4 --accel tcg --timeout 360 --cmd-timeout 180
```

The host tests compile the actual registry, access and SpinLock code with only
IRQ/context and small owned task-record fixtures. Original compile-negative
lifetime/static-argument cases remain. Eight host threads complete 8,000 mutations
on one task and query metadata from inside the callback. Pinned removal and
replacement must delay destruction; destructors reacquire metadata to detect
wrong drop order. A callback already admitted to the retired object finishes
without touching its replacement. A separate blocked lookup must leave registry
available, then reject its stale Arc after replacement before invoking callback.

The native TASK-REGISTRY smoke runs against real Process, VFS, FPU and VM objects.
A callback queries snapshots, removes its own registry record, verifies the
pinned object/fd remains usable and returns. Weak task/FPU owners must then expire,
the VFS descriptor must be closed and the open-file count restored. A pending
real dispatch frame must likewise keep a removed task and its address space alive
until frame drop. Synthetic user instructions are never executed by that smoke;
real apps, faults, preemption and FPU/TLS scenarios exercise actual HAL entry.

The original regression budgets are retained, with an added required task proof.
Native/image receipts and fixed-work before/after samples are in
[the measurement record](process-registry.json). C pointer verification checks
actual cursor pixels, lock/confine/release and exit cleanup, not visual judgement.

Eight final native runs passed: resource and fault/lifecycle scenes, the 1/4/8
vCPU hardware/PMM/registry matrix, GUI, C pointer and production. The production
contract also forbids the feature-gated registry and PMM smoke markers.
The same 36 fixed-work samples retain identical work counts and checksums.
Before/after medians were 50/50, 40/40, 50/50 and 50/50 ms for CPU scenes,
260/230 ms for allocation and 30/30 ms for memfs I/O. These are diagnostic values:
the host environment restarted between runs, TCG was used and the guest clock
has 10 ms resolution with interrupt-disabled undercount. They do not establish
a speedup or application SMP scaling.

## Remaining gates

These locks/pins do not make Ready→Running(cpu), block/wake versus HAL return,
active execution cancellation or handle transactions atomic across CPUs. A frame
can retain a retired object; that gives no right to start/restart that object.
Execution ownership, stopped-task handoff, remote wakeup and IRQ/preemption/idle
progress remain open. Mutex-protected fields do not exclude asynchronous hardware
FPU use or protect mappings from unmap/explicit VM-resource extraction. Active CPU
masks, acknowledged TLB shootdown and final shared-frame/DMA readers are required.
Terminal/global driver and other shared paths still need the full SMP audit.
APs perform isolated hardware/PMM probes and park; apps/pthreads remain UP.
No syscall/rights/GUI wire/filesystem format changes or SMP speedup are claimed.

Follow-up (14 November 2026): [exclusive execution leases](task-execution.md)
add CPU/ticket admission and return/retirement gates. The remaining-work section
above records this change's original boundary, not a claim that later admission
work closes cancellation, remote scheduling or mapping/TLB lifetime.
