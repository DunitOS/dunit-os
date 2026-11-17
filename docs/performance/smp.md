# SMP foundations and remaining safety gates

Current state: 16 November 2026. Applications and pthreads execute on BSP:
`scheduler=up dispatch_cpus=1`. APs initialize private hardware state, run
isolated hardware, PMM and TLB probes, then park. More online CPUs do not
establish parallel applications or a speedup. OP0–OP3 remain partial; OP4
application dispatch is not accepted.

This document describes the current invariants. Linked JSON receipts retain the
tested source/image versions, commands, failures and historical boundaries.
Older receipts can list prerequisites that subsequent changes implemented.

## CPU-local entry

Limine publishes AP entry points after BSP prepares private CPU storage and
shared kernel mappings. Each AP loads kernel CR3 and its own stack, initializes
x87/SSE, GS, GDT/TSS/IST, syscall MSRs, IDTR and local APIC. The published IDT is
read-only and shared. Logical CPU indices differ from hardware APIC IDs. Capacity
is 64 CPUs; larger configurations fail explicitly. AP descriptors and stacks are
allocated before publication and remain permanent; hotplug is outside this work.

One schema generates C/NASM CPU-local offsets and Rust capacity, checked by C
layout assertions and build freshness checks. Syscall scratch, selected/escape
stacks, FPU pointers and current-task/exit/yield/schedule flags are CPU-private.
Paranoid interrupt entry restores kernel GS through NMI transition windows;
user transitions restore user GS. FSGSBASE is disabled to protect the anchor.
FS/TLS and FXSAVE context handling remain; XSAVE/AVX is a separate gate.

BSP retains PIC device routing through APIC virtual wire and PIT events. AP
timers are calibrated against PIT, checked, then masked. Parked APs use HLT and
remain IPI-wakeable without periodic timers. Identical pending APIC vectors can
coalesce, so probes serialize request/ack round trips with release/acquire
publication. xAPIC was tested; x2APIC is not fully validated.

[Bring-up receipts](smp-bringup.json) cover BIOS 1/2/4/8 CPUs and UEFI 4 CPUs.
Every AP executes a checked 65,536-iteration sum, receives at least two local
timer ticks and completes a two-way IPI exchange. Negative report tests reject
fabricated CPU identities/progress. These are hardware checks.

## Task ownership

Registry metadata belongs to `IrqSafeSpinLock<Vec<ProcessRecord>>`. Each record
owns an independent `Arc<IrqSafeSpinLock<Process>>`. Lookup clones the Arc under
metadata, unlocks metadata, acquires the task lock, then revalidates membership
and Arc identity. A waiting old lookup cannot mutate a replacement PID. A
callback already admitted to a retired object may finish on its pin without
affecting a replacement; this does not grant cancellation semantics.

Current owner/thread and explicit-PID scopes use higher-ranked callbacks. Only
owned results escape; Process and field borrows cannot. There are no static
mutable registry/current-task getters. Metadata snapshots never take task locks.
Signals copy source authorization and target PID before entering the target
mutation scope. Nested task locks remain forbidden.

Lifecycle Ready/Blocked differs from execution ownership. Each record has a
CPU-bound, generation-tagged lease:

```text
Idle -> Reserved(cpu, generation) -> Active -> Returned -> Idle
```

Preparation reserves the record while copying context/backing. Before touching
CR3/FPU/TLS or entering HAL, the lease validates task identity, owner lifecycle,
Ready state, ticket and CPU, then publishes Running/has_run. Outstanding leases
exclude reentry even after yield, block, wake or preemption publishes Ready.
HAL return resets TLS/FPU/syscall-stack policy before Returned; exit/fault/TLS
cleanup completes before releasing admission. Release retries Ready admission
after guards end. Stale tickets cannot start or release replacement tasks.

Group kill checks all owner/thread execution slots in one metadata transaction.
Reserved siblings cannot enter after death. Entered siblings cause the existing
NotRunnable/EAGAIN refusal rather than active backing reclamation. Owner exit,
reap and join similarly respect outstanding ownership. Remote stop/cancel
acknowledgements remain open; refusal does not implement them.

The ready queue owns only PID/generation records and its cursor. Validation and
scheduler callbacks happen outside its lock. Bounded traversal allocates no
snapshot; stale removal/confirmation cannot affect a newer same-PID entry or
skip its successor. Queue observations are rechecked by execution admission.
Remote idle/check-sleep/reschedule ordering is still absent.

## Dispatch lifetime

DispatchFrame retains task, opaque kernel-stack bytes, aligned interior-mutable
FPU storage and AddressSpace owners alongside copied registers and FS base.
No task/metadata/mapping lock or IRQ guard spans HAL/userspace execution.
Userspace entry rejects an IRQ-masked caller.

DispatchScope enters the root under its mapping mutex, publishes a CPU-bound
active pin, and retains the previous root until switching away. Drop restores
caller CR3, PID and syscall-stack policy on normal/error paths. Canonical kernel
half refresh runs even when CR3 repeats. Caller backing stays owned throughout.
Frame drop alone cannot retire a root still active on a CPU.

Fd, VM, capability and task cleanup batches are detached under short mutation
scopes and destroyed after guards end. Kill closes pipe writers/fds before parent
wait. Process Drop drains residual ownership. Inheritance constructs a complete
owned fd set before swapping: VFS retains once per distinct open description,
pipes per descriptor, and PTY/status Arc identity remains shared. Partial setup,
missing child and replacement roll back through owned batch Drop.

## Waits and IPC

IPC, pipe, poll and wait-set capture a notification checkpoint before testing the
producer predicate. Admission compares it under task/metadata/registration locks.
Every producer advances the generation even without waiters. A producer winning
the gap causes an existing syscall restart/Ready transition without parking;
the execution lease prevents reentry before HAL returns. Expired deadlines also
avoid parking. Conservative global notifications may retry unrelated waits.

Foreground exec observes Ready/Blocked together under the registry lock. An IRQ
wake between separate queries could previously reject a live waiting process as
an ELF launch failure. Host checks reproduce that gap and exercise 50,000 locked
observations against concurrent transitions; non-waiting or missing tasks fail
the retry predicate. Execution admission remains a separate check.

Wake selection removes a generation-tagged registration under its own lock,
then changes task state and admits it after releasing guards. A pass is bounded
by its initial generation, preserving FIFO/futex limits without a temporary Vec.
Delayed old callbacks cannot wake a new registration of the same PID. Generation
exhaustion fails explicitly. Cancellation detaches registration/pending wait-set
under task -> metadata -> queue; selected callbacks still require identity checks.

Futex admission follows task -> AddressSpace -> metadata -> registration. Atomic
Acquire word comparison and Blocked/register commit stay serialized with wake.
The private UserWord borrows the mapping guard, verifies alignment/PRESENT/USER,
and exposes owned values only. Clear-child-tid requires WRITABLE, uses Release
store, unlocks mappings, then notifies. Page-end aligned words are valid;
PROT_NONE/unmapped words are rejected. Sleep has no producer predicate.

IPC uses a real IRQ-safe guard and bounded VecDeque FIFO. Byte send keeps a live
target task; task -> IPC -> metadata validation orders send with retirement.
Capability insertion and publication share the target transaction; rollback and
notifications run outside IPC/task guards. Attached receive copies an owned
snapshot, then commits only if its unique sequence still identifies the exact
head. A stale receiver cannot pop the next packet. No IPC guard spans user copy.

## Memory and TLB

PMM bitmap scan/reserve/clear and counters share one IRQ-safe leaf lock. A separate
initialization lock transfers static bitmap backing once; repeated/concurrent
init cannot reset live allocations. First-fit single/contiguous allocation,
tail-bit reservation, constructor bounds and unaligned-free rejection remain.
Heap/AddressSpace/device owners may acquire PMM; PMM never allocates, logs,
maps, enters process callbacks or waits for peers. Contiguous scanning remains
a coarse IRQ-off section. Freeing still requires final reader/DMA/mapping ownership.

Actual BSP/AP probes perform 128 simultaneously live mixed allocation cohorts
per CPU, check every frame identity before any frees, and restore PMM stats.
Entry/barriers are bounded and release peers on failure; no PMM guard spans a
barrier. They use no heap/VFS/process/mapping path and return APs to HLT.

AddressSpace and MMIO transactions use real locks. Page-table walks use shared
references and atomic 64-bit entries, including hardware accessed/dirty writes.
The canonical shared kernel hierarchy is installed before user roots. Atomics
do not replace mapping ownership or invalidation.

Each CPU publishes its active root before CR3 load. Mutation precedes a
sequentially consistent active-root snapshot. User invalidation targets that
root's CPUs; kernel alias invalidation targets every online CPU. A serialized
sender publishes exact generations in per-CPU mailboxes, fences before APIC
delivery, sends NMI and waits for acknowledgement. The NMI receiver uses private
IST/GS/SIMD state, reads CR3, INVLPGs where required and acknowledges without
task, mapping, MMIO, heap or sender locks, including with ordinary IRQs masked.

Unmap acknowledges before frame reuse, protection before return, alias removal
before virtual extent reuse. Rejected delivery/missing ack panics before reclaim.
Fresh not-present mappings do not broadcast; replacement first unmaps. User GLOBAL
mappings are rejected. Root destruction asserts no active CPUs.

User-memory views are CPU-bound and cannot escape. Mapping locks span validation
and every byte/pixel-row copy. Concurrent unmap waits; only owned data leaves.
Active dispatch pins avoid current-task relocking. Pixel rows preserve volatile
forward word copies and unaligned fallback. Thread return slots initialize before
Ready. Production access without an active dispatched mapping returns EFAULT;
only explicit boot fixtures use the permanent bootstrap root.

## VFS and PTY

VFS registry guards are scoped and CPU-bound. Mounts, open descriptions and file
capabilities retain the same Arc-owned locked backend; root/direct/mounted access
has one authoritative lock. Backend lifetime survives namespace deletion and
unlink until final close. Append seek/write and private-map seek/read/restore are
single backend transactions. Path scratch is per operation; rename keeps both
paths on stack. Task metadata snapshots avoid holding task locks across callbacks. Boot publication
is one-shot; concurrent hotplug/unmount and persistent format changes are outside
this work. Coarse VFS locks do not establish throughput scaling.

PTY table transactions retain a real IRQ-safe guard through ring/geometry access.
Slave I/O/ioctl/readiness holds a privately constructed owning endpoint throughout
the operation, including user copy. Master metadata checks happen outside PTY
locking and preserve EOF if detached resource cleanup is delayed. Notifications,
including newly freed input capacity, and removed channel destruction happen
after unlock. Attachment generations prevent an old final destructor deleting
a replacement; endpoint moves retain identity and narrowing/wrap are checked.

Close, dup2/install replacement, invalid/full fd allocation, capability close and
terminal binding retire outgoing owners into a task batch. The common scope
unlocks the task before destruction/readiness, including callback errors and
empty stdio paths. State/context registry callbacks perform no fd operations.
These boundaries do not make arbitrary task callbacks reentrant.

## Pre-OP4 gates

The global VMM getter was removed in the acknowledged-TLB implementation.
Boot metadata is atomic; MMIO extents and kernel table publication share a lock,
and process mappings use owning address-space scopes. Active-root mutation
helpers are restricted to early boot smoke tests. This closes the old getter
item, not the concurrent shared-VM/final-reader acceptance gates.

Console history now belongs to the console under one spinlock. Shell and
foreground output use a handle whose operations release the guard before return;
no console borrow/lock spans shell commands, VFS work, stdin polling or HAL entry.
Serial output runs outside that lock and long output releases it between bounded
character batches. Foreground PID/sink and the stdin owner/line/readiness share a
separate lock. Ancestry checks precede acquisition and revalidate the foreground
selection; notifications follow unlock. Partial reads retain their owner, a
competing request cannot replace unread input, and a buffered line is not a new
keyboard request. Foreground switches discard pending input atomically.

`kernel/tests/terminal_stdin.rs` checks 10,000 concurrently supplied lines with
one-byte reads, owner isolation, stale supply and reset. `terminal_display.rs`
checks independent history, concurrent handles/snapshots, serial reentry and the
existing display lease. `tools/boot_markers.json` additionally drives two real
`calc` input lines, normal exit, Ctrl+C and a subsequent successful exec under
TCG on one/eight CPUs. These checks exercise UP application dispatch; APs park.
Display leases, framebuffer metadata and the active backend now share a device
spinlock. Console pixel operations hold it for one row/glyph/cursor; no framebuffer
pointer is cached in the console. Layout refreshes after modesets and each write
uses the locked current pitch/bounds. Console and display IRQ handlers never
acquire these locks; timer preemption switches only CPL3, so kernel-shell rendering
leaves IRQs enabled. `console -> display` and `task/VM -> display` are the relevant
edges; source mapping pins precede display acquisition, and present/flush share
that scope. Serial logs and user descriptor copies run after display unlock.

Every acquired display handle contributes a lease; the last close releases the
owner. The selected Bochs backend owns its full-VRAM mapping for its published
kernel lifetime. Modesets keep that backing, validate physical/virtual geometry
and offsets, and publish dimensions/pitch together. A rejected programming attempt
restores prior registers; failed rollback removes the visible framebuffer while
retaining backing until a valid recovery modeset. GetFramebuffer keeps the native
24-byte record and initializes its padding. Rectangle clipping bounds work to the
visible area before looping.

The host display tests use actual locks/Bochs programming with port/VRAM fixtures:
2,000 concurrent modesets, 6,000 pinned copy scopes, delayed writer handoff,
multiple leases, canaries, rollback and failed-rollback recovery. Native handle
checks cover acquire/close, record padding, copy errors and revoked writes; GUI
checks cover live resolution reload and the 3K preset/screendump sequence.
On 15 November, the boot suite passed with one/eight TCG CPUs, GUI smoke passed
with four CPUs, and the production kernel (without `boot-smoke-tests`) passed
native TinyCC core/IPC workloads and musl time checks. The real Settings sequence
passed 3072×1920 → 2880×1800 → 2560×1600 → 2560×1440 → 1920×1080; every P6
capture had the selected dimensions, a complete pixel payload and a nonblank
desktop. Its coordinates now target the retained Settings controls and each
capture waits for the new session's first presented frame, rather than accepting
a black image immediately after hardware modeset. Requested harness screenshots
also fail the run on QMP rejection or missing output; host transport/run tests
cover those failures. ABI and CPU-local generated outputs remain unchanged.
These cover the current Limine/Bochs mechanism. They do not establish AP application
dispatch, GPU DMA/hotplug retirement or syscall IRQ-off latency budgets. Full-frame
user mapping/copy and syscall interruptibility still require the IRQ-progress gate.

Input-master handles now retain counted leases; owner, generation, pointer facts,
display bounds and event queue share one IRQ-safe lock. A repeated acquire keeps
the current stream. Final close clears it atomically; the next owner receives a
fresh LOST_BEFORE baseline. Exit/kill cleanup retires each outstanding handle.
Handle authorization captures a generation under task metadata; output mapping
pin/validation precedes input acquisition, then generation revalidation, fixed
32/40/56-byte copy and pop share the device scope. This removes input -> task/VM
edges and rejects delayed reads across same-PID reacquisition. Keyboard modifier
snapshots happen before input acquisition; display -> input only publishes bounds.
Legacy keyboard/snapshot calls retain their previous access policy. The 16-byte
GetMouseState compatibility adapter now pins output before input, copies one
coherent state and drains its wheel accumulator only on success; EFAULT preserves
wheel input and neither result consumes the native event queue.

Host tests use the actual SpinLock with platform IRQ behavior replaced. They
check 2,000 contended lease epochs alongside 5,000 device updates, coherent
positions, delayed-copy/final-close handoff, failed copies and stale same-PID
tokens. Native pointer checks retain short clicks and Shift/Ctrl snapshots while
sleeping, verify denial to a competing child after the first close, and reacquire
after exit/kill of a child holding two handles. Host and native wheel injection
also cover failed compatibility copies and exactly-once wheel drain. These narrow device transactions
do not make end-to-end syscalls interruptible or establish remote application
execution/cancellation safety.

Block/device registries now use one lock each. Registration publishes VFS nodes
after unlocking; I/O copies the callback before dispatch. PCI CF8/CFC pairs and
BAR probes are serialized. Command-register writes preserve W1C status bits.
Initrd publishes one immutable parsed archive; repeated initialization does not
replace storage held by readers. Unused userspace keyboard/mouse drivers were removed.

AHCI keeps a locked disk table and marks uncertain commands offline before any
retry can rewrite descriptors/data. Initialization failure frees DMA only after
observing both CR and FR stopped; a failed stop retains the pages until reboot.
VirtIO owns a contiguous queue, observes reset, validates used-index/head identity,
and takes timeout/invalid-completion devices offline with all backing retained.
Its full-range LBA check precedes the first sector submission. Neither path
claims recovery or allows a late completion to re-enable the device.

The 19 host storage regressions cover AHCI busy-TFD timeout, delayed/late CI
completion, active-command task-file errors, failed CR/FR stop and exact page
release; VirtIO late/stuck/invalid completion, stuck reset, PFN-zero ownership,
full-range rejection, read-only negotiation and invalid-queue/OOM failure;
PCI contention/BAR restoration, callback reentry and immutable initrd publication.
VirtIO failure status preserves earlier handshake bits. A read-only device rejects
writes before touching DMA, and the block registry refuses its write callback.

On 16 November, native AHCI and VirtIO install/rollback/fsck suites passed with
four TCG CPUs. Separate stop/start VMs verified 131072 bytes with hash
`b4c071280fb62325` and clean VFS handles on both buses. Logs and inspected frames
are in `build/review-nov16/{ahci,virtio}-persist-{write,verify}`.
The production read-only VirtIO scenario
showed `ro`, rejected block writes and installation, and read the original bytes
again. The complete 256 MiB image SHA-256 remained unchanged; screenshots and
serial are in `build/virtio-readonly` and `build/review-nov16/readonly-current`.
Hardware recovery, full lock order
and IRQ progress stay open.

The following require implementation or further native proof before AP application
dispatch:

- Remaining driver shared-state owners, complete task/VFS/backend/device/heap/
  PMM/IRQ lock ranking, and bounded IRQ-off syscall/frame-present work.
- Interruptible syscall/IPI progress, remote cancellation/group-stop
  acknowledgements and cleanup only after active CPUs hand back execution.
- Remote runqueues, reschedule/idle notifications, migration and wake/check-sleep
  ordering with real overlapping userspace execution.
- Remote CPL3 faults/W^X, concurrent shared mappings/process exit and final
  shared-frame/DMA readers; isolated TLB/allocator probes are narrower proofs.
- A parallel application regression matrix and a stable KVM/reference platform
  for performance acceptance. Later OP stages and latency goals remain open.

Lock order currently established: lookup releases metadata before task; mutation
uses task -> AddressSpace when needed -> metadata -> wait registrations; VFS
uses registry -> backend. Metadata never waits for task/AddressSpace/VFS, runs
external callbacks or destroys resources. Leaf heap/PMM and registration-storage
allocation are explicit commit exceptions requiring the full ranking above.

## Verification

Historical host tests compile actual modules with privileged instruction/transport
shims. Guard tests reject migrating IRQ/lock guards and sharing guards for non-Sync
data, and perform 80,000 concurrent updates. Registry/access tests preserve
non-escaping borrows and 8,000 concurrent mutations, stale leases, pinned
retirement and reentrant destructors. Execution tests check 8,000 admissions and
another 8,000 state cycles without overlap, stale CPU/ticket/replacement identities,
and compile rejection of migrated leases. Queue tests check 8,000 requeues or
registrations; parking forces 8,000 producer gap wins and delayed callbacks.

PMM host checks verify 80,000 mixed transactions/239,984 frame handbacks; the old
actual module fails duplicate-live-frame ownership. IPC checks 8,000 ordered
packets plus errors/stale commits/reentry. TLB checks 8,000 transactions/56,007
acks including delayed/missing delivery; user-copy checks reject escaped/migrated
views and hold off unmap. PTY checks 100,000 ordered bytes in each direction,
capacity/EOF/replacement and reentrant wake/metadata hooks.

Native receipts retain boot/DF/IRQ, 15 positive resource scenes and a separate
intentional fault/join VM, 1,000 joinable plus 1,000 detached TLS/errno/allocation
cycles, GUI resize/recycle/cooked-shell and exact C pointer pixels. The TLB AP
probe performs 32 IRQ-off permission/unmap/retarget/alias-reuse rounds, explicit
left-generation checks and final PMM accounting. Production budgets forbid
boot-only proof markers. The latest PTY change tested BIOS/TCG, not UEFI/KVM;
the earlier bring-up receipt contains the UEFI proof.

Receipts: [VFS](vfs-ownership.json), [resources](process-resources.json),
[scheduler queue](scheduler-queue.json), [public task access](process-access.json),
[dispatch](dispatch-ownership.json), [wake queue](wait-queue.json),
[futex](futex-admission.json), [internal scopes](task-scopes.json),
[registry](process-registry.json), [execution](task-execution.json),
[PMM](pmm-concurrency.json), [wait/clock/IPC](progress-foundation.json),
[TLB/user copy](tlb-shootdown.json), [PTY/fd retirement](pty-ownership.json).

Native OS tests run through tools/qemu_test.py or its existing focused wrappers;
see [reproduction](measurements.md#reproduction). Receipts are prior evidence,
not a claim that this documentation edit reran those tests.
