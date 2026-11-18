# Dunit OS performance and SMP roadmap

Updated 17 November 2026. OP0–OP3 are partial; the next stage is to finish OP3
safety gates before OP4 application dispatch. This track complements the
[technical roadmap](DUNIT_OS_TECHNICAL_ROADMAP.md); it does not close M7.
Implementation details and evidence are in [SMP](docs/performance/smp.md),
[measurements](docs/performance/measurements.md) and
[GUI performance](docs/performance/gui-performance.md).

## Goal and current limits

Reduce useful-work time, input latency and frame-time variation; improve
throughput with more CPUs at identical workload and image quality. First targets
are 1/2/4/8 vCPUs, desktop/text/scrolling/IPC/files and Rust/C CPU workers. Add a
fixed software-rendered game benchmark; include Minecraft only after a
reproducible build and scene exist. CPU/SMP is the first path; measure the
software renderer's limits before proposing GPU acceleration. NUMA, CPU hotplug
and hard real-time guarantees are outside this first pass.

The verified TCG matrix boots up to eight CPUs, but applications remain
`scheduler=up dispatch_cpus=1 APs=parked`. APs run isolated hardware/PMM probes.
CPU count, AP boot and `online=4` do not prove simultaneous useful execution.
KVM is unavailable on the current runner. q35 uses validated HPET with a 10 ns
counter step and PIT fallback; timer events are still PIT 100 Hz. Counter
resolution is not wakeup latency. Historical PIT elapsed and host/QMP stream
measurements do not establish per-event tails or input-to-photon latency.

| Stage | Status | Remaining acceptance |
| --- | --- | --- |
| OP0 | Partial | KVM runner, remaining scenes, full telemetry, budgets/noise |
| OP1 | Partial | Remote lifecycle/cancellation and IRQ/preemption/idle model |
| OP2 | Partial | Timer event model, x2APIC/KVM/hardware platform coverage |
| OP3 | In progress | DMA fault coverage, remote stop/cancel, IRQ-off budgets, lock ranking, concurrent VM stress |
| OP4 | Prepared only | AP application dispatch, remote wakeups, migration and overlapping execution |
| OP5 | Not started | Scheduler scaling, accounting and latency |
| OP6–OP8 | Open | New profile-driven input, rendering, memory and I/O work |
| OP9 | Open | Final correctness/scaling/performance and regression matrix |

A completed subtask below is a narrow implementation with evidence. It does
not imply acceptance of its whole stage. `[x]` marks that completed subtask;
`[ ]` marks work or acceptance that remains open.

## 17 November checkpoint

`e74aebc` adds the post-cleanup exit-publication fence; OP3 remains in progress.
The parallel SDK track (`c08a900`) adds owning threads/mutex/condition APIs, and
the networking track (`80c53ef`) proves private E1000 PHY-loopback DMA. These
changes do not enable AP application dispatch or establish performance scaling.
[Integrated receipts](docs/verification/multitrack-2026-11-17.json) record native
SDK on four CPUs, E1000 boot on one, strict resources/SDK on eight, thread faults
on four and GUI on eight, with separate image hashes and repaired attempts.
The next parallel slice adds [interruptible syscall return boundaries](docs/performance/irq-progress.md)
and a generation-tracked ordinary reschedule IPI. Native 2/4/8-vCPU TCG checks
verify PIT/IPI delivery inside a syscall, IRQ-safe-lock exclusion, post-HAL
acknowledgement and userspace GPR/SIMD/TLS preservation. This is a progress
prerequisite, not remote cancellation or AP application scheduling. The network
track also verifies private external INTx packet exchange; SDK distribution
checks include offline relocated application builds. [Follow-up receipts](docs/verification/multitrack-progress-2026-11-17.json)
separate these results and the SDK native acceptance.
Next OP3 work remains remote stop/cancel, bounded IRQ-off driver/heap work and
the full lock-order/VM/DMA final-reader gates listed below.

## Measurement contract

Record commit, source/image hashes, toolchain/profile, image, QEMU version and
command, explicit accelerator, CPU model/topology, RAM, resolution/effects,
scene/seed, workers, host CPU/quota and correctness assertions. Distinguish
requested, emulated, kernel-online and dispatch CPU counts. KVM and TCG are
separate series; do not compare with acceleration set to `auto`.

Measure wall time, CPU time, ready-queue/IPC/FrameDone waits, input→processing→
publication, frame distributions, p50/p95/p99, missed deadlines, queue high-water
and loss, copied bytes/pixels, allocations, cache hit rate and RSS. Nested elapsed
stages are not independent CPU costs. Mark unavailable metrics unmeasured.
Use warmup and at least five measured repeats, retain raw results and quantify
noise; increase repeats and report intervals when needed. Aggregate opt-in
profiling, measure its overhead and avoid per-event serial output in hot paths.

For fixed work use `S(N)=T(1)/T(N)` and `E(N)=S(N)/N`; report throughput of N
independent jobs separately. TCG establishes correctness/fallback; scaling
claims require KVM or hardware with enough host cores.

Proposed OP0 budgets, still unachieved: on a fixed KVM platform at 1920×1080,
60 Hz / 16.67 ms per complete desktop frame, p95 input→publication ≤33 ms and
p99 ≤50 ms during defined normal UI plus CPU load. Freeze scene, refresh rate
and allowed missed-deadline fraction in OP0. Publication is not physical scanout.
Games, blur and larger resolutions need separate budgets. Initial independent
CPU-work targets are ≥1.5× on two CPUs and ≥2.5× on four; measure eight separately.
These targets do not apply to inherently serial apps.

## OP0 — reproducible baseline and instrumentation

- [x] Record GUI wheel/short-field/long-field/selection-drag/wheel-burst benchmarks.
- [x] Run equal-work CPU/pthread workers on 1/2/4/8 CPUs with checked data/checksums.
- [x] Measure allocator and MemFS I/O workloads.
- [x] Retain metadata/JSON provenance and distinguish PIT, host-wall and unmeasured metrics.
- [x] Record checked queue/dispatch before/after series.
- [x] Run native two-process IPC ping-pong with warmup and five 1,000-round-trip trials.
- [ ] Add idle/startup, loaded/multi-client and fixed-game scenes.
- [ ] Establish the KVM reference machine, noise estimates and numeric budgets.
- [ ] Measure profiler overhead, CPU accounting, per-event latency tails,
  queue/wait telemetry and RSS. Existing correctness tests are not these baselines.

Acceptance: one harness command reproduces equal data/pixels/counters, raw
repeats and noise. A TCG baseline is usable while KVM remains unavailable;
the KVM performance gate stays open.

## OP1 — shared-state audit and CPU-local entry

- [x] Inventory shared state, locks and resource lifetimes.
- [x] Make PID, syscall scratch, stacks/escape pointers, scheduling flags and FPU
  pointers CPU-local.
- [x] Use private GDT/TSS/IST/AP stacks and check FS/TLS/SWAPGS/syscall/IRQ/
  exception/return paths.
- [x] Bind guards to their CPU and enforce the correct Sync bound.
- [x] Scope public/private task access and defer fd notifications after locks.
- [x] Copy HAL context, own dispatch resources and pin registry/task lookups with
  identity revalidation and retirement outside guards.
- [ ] Complete remote execution/cancellation and IRQ/preemption nesting/idle semantics.

A task pin keeps storage alive; it does not grant Running ownership or permit
execution of a retired frame.

Acceptance: explicit owners/invariants, green UP regressions and no shared
writable entry scratch/stacks, then closure of all ownership gates before AP
application execution. An audit alone does not fix its listed races.

## OP2 — AP boot, APIC, IPIs and clocks

- [x] Complete Limine MP handshake and AP-local GS/GDT/TSS/IST/IDT/syscall MSR
  and x87/SSE setup.
- [x] Separate logical CPU indices from hardware APIC IDs.
- [x] Verify xAPIC/IPI round trips, EOI/spurious handling, BSP PIC routing and
  PIT-calibrated local timer probes.
- [x] Boot BIOS with 1/2/4/8 CPUs and UEFI with four CPUs under TCG.
- [x] Park APs after probes without perpetual timer wakeups.
- [x] Validate global HPET ACPI/checksum/64-bit width/progress, including IRQ-off
  and AP brackets, without assuming synchronized cross-CPU TSC.
- [x] Verify PIT fallback and separate clock/event metadata.
- [ ] Define precise timer events/wakeups and add x2APIC/KVM/real-hardware coverage.

Acceptance: AP boot/park and work/timer/IPI checks on each supported platform,
with clock/event limits stated. This stage does not establish an SMP scheduler.

## OP3 — memory, ownership and synchronization

- [x] VFS operation-local paths, both rename endpoints, owning locked backends,
  scoped VFS access and final-close lifetimes; raw backend pointers removed.
- [x] Detached fd/VM/capability cleanup batches, fd inheritance rollback and owning
  snapshots. Kill closes fd/pipe resources before wait; releases occur outside
  lifecycle mutation.
- [x] Queue→task-table lock edge removed; generation-safe validation/requeue and
  bounded allocation-free dispatch traversal. Current/explicit-PID access and
  fd rescans use non-escaping scopes, including errors.
- [x] Arc-owned stack/FPU/locked address space, copied HAL input, active-root pins,
  canonical kernel-half refresh and caller PID/CR3/stack restoration on errors.
- [x] Wake callbacks outside wait-queue locks; generation cutoff, scheduler admission
  after task borrow and futex lookup/mapping validation before queue acquisition.
  Futex compare/register stays atomic; clear-child-tid uses Release.
- [x] PMM leaf-lock transactions for allocation/contiguous/free/accounting, one boot
  bitmap handoff, host mixed-transaction stress and BSP/AP probes on 1/4/8 CPUs.
- [x] Registry/task locks and pins across lookup, TLS/join/reap/kill, preemption and
  futex, with post-lock identity validation and no mutex held through HAL/IRET.
- [x] Notification checkpoints and park identity for IPC/pipe/poll/wait-set producer
  gaps; delayed wake cannot affect a later park. Locked IPC FIFO, notifications
  after guards and atomic handle insertion/message publication.
- [x] Acknowledged TLB invalidation: active roots, generation/ack, lock-free NMI
  receiver, user unmap/protect and kernel alias reuse. Address-space mapping pins
  protect validation through copying; atomic PTE operations avoid mutable whole
  page-table references. Isolated AP retarget/reentry/reclaim and dropped-ack
  refusal are tested.
- [x] Locked PTY table, owning endpoint snapshots for I/O/ioctl/readiness, attachment
  identity across final-drop/reattach, notifications and last endpoint/file
  owners retired after task scopes on close/dup/install/bind/error paths.
- [x] Global VMM ownership: atomic boot metadata, locked MMIO extent/kernel-table
  publication and scoped owning address-space access. The old VMM getter was
  removed by the acknowledged-TLB work; active-root mutation helpers are boot-smoke-only.
- [x] Console/scrollback ownership ends at each output, cursor, theme or scroll
  operation; shell/exec retains only a handle. Foreground selection, stdin owner,
  supplied bytes and readiness share one lock; partial reads retain ownership,
  competing requests cannot steal a line and wake callbacks run after unlock.
  Host concurrency and native interactive/interrupt/re-exec regressions cover this scope.
- [x] Observe foreground Ready/Blocked retry states under one registry guard;
  IRQ wakeups cannot fall between separate queries and reject a waiting exec.
- [x] Serialize display leases, scanout geometry, pixel writes and modesets under
  one device lock. Console rendering uses current pitch and bounded row/glyph
  scopes; kernel-shell rendering keeps IRQs enabled. Repeated display acquisition
  retains ownership until the last handle closes. The backend owns its boot-lifetime
  VRAM mapping; rejected modes restore registers or quarantine scanout after failed
  rollback. Host contention/fault injection and native lease/GUI modeset checks cover
  the current Limine/Bochs path.
- [x] Serialize input-master leases and pointer facts/queue under one IRQ-safe
  lock. Repeated acquisition retains unread events until final close; exit/kill
  retires all leases. Task-scoped generation validation, a prior user mapping pin
  and bounded copying prevent stale reads from draining a later session or
  acquiring VM/task locks under input. Host contention and native pointer/GUI
  regressions cover the current mechanism without changing input ABI or policy.
- [x] Lock block/device registries, publish nodes and invoke I/O after unlocking;
  serialize PCI address/data pairs and preserve W1C status during BAR probes.
  Publish immutable initrd contents once and remove unused userspace input drivers.
- [x] Take AHCI/VirtIO offline after uncertain completion; retain DMA backing and
  reject retries. AHCI initialization frees pages only after observed CR/FR stop;
  VirtIO observes reset, owns contiguous queues and validates completion identity.
  Host failure tests and native install/fsck/stop-start persistence cover these slices.
- [x] Inject AHCI delayed/late CI completion and active-command task-file errors
  in host tests; verify no retry rewrites backing or releases live DMA pages.
  VirtIO read-only negotiation and invalid-queue/OOM failure paths are also covered.

- [x] [Fence exit publication through cleanup](docs/performance/op3-lifecycle.md):
  revoke unused execution admission, exclude wait/reap/replace during finalization,
  release fd/TLS/VM/HAL backing before publishing status, and keep private TLS
  cleanup idempotent without weakening public munmap. This does not stop a remote
  active executor or close the remote cancellation gate.

- [x] Enable interrupts only after syscall scratch/FPU state and kernel stack are
  established, then close an IRQ-disabled return boundary after handler guards
  unwind. Ordinary reschedule IPI generations distinguish receipt from actual
  HAL handback; CPL0 IRQs and all NMIs return to their interrupted context.
  Native probes preserve syscall results, GPR/SIMD/TLS and lock exclusion.
  This does not authorize remote stop/cancel or reclamation.

- [x] Bound framebuffer source validation to one-page scopes and source copies
  to 4 KiB per mapping pin, release the VM guard before scanout writes, and
  retain display owner/geometry through a separate access scope. Other syscall,
  driver and heap IRQ-off work remains open; see [present ownership](docs/performance/framebuffer-progress.md).

Remaining gates, in order:

- [ ] Bound end-to-end IRQ-off syscall/frame-present work and complete remaining
  driver/heap edges. Interruptible handler entry alone does not bound every
  IRQ-safe critical section.
- [ ] Remote cancellation/stop acknowledgements and group finalization. Publish
  waitable exit only after executors stop and cleanup completes; cover nested
  dispatch, join/detach and cancelling waits. Check/register identity alone
  does not stop a running CPU.
- [ ] Complete lock ranking/reentry for registry/task, VFS/task snapshots, waits, IPC/handles,
  heap/PMM/VMM and devices. Never sleep under a spinlock or wait for an ack while
  holding a lock the receiver needs. Extend the verified syscall/IPI boundary to
  bounded IRQ-off sections and remote runqueue notifications.
- [ ] Prove heap/PMM/shared-frame/handle/device order and final-reader lifetime.
  Address-space/task/execution pins are different contracts; no one pin replaces
  mapping ownership or shared-frame bookkeeping.
- [ ] Real remote CPL3/shared-VM/W^X fault, concurrent exit, migration and page-reclaim
  stress, with fault injection. Existing isolated AP probes and UP regressions
  do not replace this matrix.
- [ ] Validate AHCI/VirtIO controller recovery and final DMA retirement on hardware.
  Host completion faults cover quarantine; they do not prove recovery.

Acceptance: no stale mappings, double frees, deadlocks or capability leaks.
General AP userspace dispatch is prohibited until these gates pass.

## OP4 — concurrent process/thread scheduling

- [x] Prepare exclusive CPU/generation/task execution reservations: enter Running
  before HAL and forbid readmission until return/cleanup handoff finishes.
- [x] Check retirement across owner/thread groups; pass actual-lease host stress
  and native UP regressions.
- [ ] Enable AP application dispatch only after every OP3 safety gate passes,
  retaining the exclusive Ready→Running(cpu) claim.
- [ ] Add remote cancellation and stop acknowledgements for concurrent executors.
- [ ] Implement remote wakeup/reschedule IPIs and race-safe block/wake/deadlines.
- [ ] Provide CPU-local idle and preserve GPR/FPU/TLS across preemption/migration.
- [ ] Demonstrate checked useful application overlap on different CPUs.

One ready queue is acceptable for the first correct scheduler; a kernel-wide
lock serializing all syscalls/runtime is not the performance goal.

Acceptance: workers/pthreads overlap on different CPUs and produce checked useful
results. On 1/2/4/8 CPUs stress shared memory, mutex/condvar/futex, IPC,
join/detach, kill/exit and FPU/TLS. Show KVM scaling against one CPU separately.

## OP5 — scheduler scaling and predictable latency

- [ ] Profile contention/migration/cache locality before choosing per-CPU ready
  queues, remote enqueue and bounded balancing.
- [ ] Add CPU accounting and wakeup→run latency measurement.
- [ ] Shorten non-preemptible/IRQ-off sections.
- [ ] Provide interactive policy without starving background jobs.
- [ ] Add idle/deadline/tickless work after timer correctness.

Acceptance: loaded GUI meets OP0 budgets, no starvation/lost wakeups or busy idle,
and scaling improves or evidence shows the shared queue is not yet the limit.
Extra timer interrupts alone are not an optimization.

## OP6 — input, IPC and SDK latency

- [ ] Measure queue→scheduling→IPC→client update/render→commit/composition,
  queue depth and burst recovery.
- [ ] Remove unnecessary wakes/copies/syscalls and batch bounded updates while
  preserving key/button ordering, wheel detents, LOST_BEFORE recovery, cursor
  modes and transfers.
- [ ] If FrameDone is a limit, define bounded frames in flight, backpressure and
  resize cancellation; published buffers remain immutable until final release.

Acceptance: p95/p99 and backlog improve on UP and SMP while exact text, selection,
undo, transfers, cursor lock, resize and frame completion remain correct.
Public protocol changes require an approved contract first.

## OP7 — software rendering and parallel SDK work

- [ ] Profile damage/occlusion, layout/diff, glyph/text caches, SVG/icons,
  ARGB/blur and memory bandwidth.
- [ ] Remove repeated layout/allocations/repaints with proven pixels and bounded caches.
- [ ] Compare release/LTO/codegen speed, size and reproducibility within image/ESP limits.
- [ ] Add checked SIMD/blits with scalar fallback; AVX requires CPUID/OSXSAVE and
  scheduler extended-state preservation first.
- [ ] After OP4 and SDK thread/sync ownership, use bounded workers for independent
  jobs/tiles. UI state/inbox/publication retain one owner; use immutable snapshots,
  correct dirty regions/blur halos and blending order.
- [ ] Separate game simulation/rasterization cost from compositor cost.

Acceptance: scalar/optimized and serial/parallel paths agree on pixels, memory
stays bounded and fixed scenes improve. Keep small jobs serial when coordination
cost exceeds useful work.

## OP8 — memory, files and I/O

- [ ] Measure allocator contention/fragmentation, cache misses/false sharing,
  shared-buffer/IPC copies and RSS.
- [ ] Apply bounded batching/locality/per-CPU caches and reclamation to observed limits.
- [ ] Optimize VFS lookup/buffering/block I/O without changing coherence, rights,
  partial I/O, errors, durability or disk format merely for a benchmark.
- [ ] Avoid global locks or IRQ-off sections spanning large copy/syscall/driver work.

Acceptance: fixed-work throughput/latency/memory improve; fsck/recovery,
persistence/reboot and memory-pressure regressions remain green.

## OP9 — final matrix and regression protection

- [ ] Compare 1/2/4/8 CPUs, KVM/TCG separately, cold/warm and idle/loaded at fixed
  scenes/effects and normal/heavier supported resolution.
- [ ] Keep short correctness/smoke CI separate from long stress/performance runs.
- [ ] Enable numeric thresholds only after stable-runner OP0 noise evidence.
- [ ] Publish before/after latency/frame distributions/scaling/throughput,
  lock/IRQ-off hotspots, RSS and limitations.
- [ ] If software rendering remains the limit, propose GPU/virtio-gpu separately
  with a software fallback.

Acceptance: SMP performs concurrent useful work and provides measured benefit,
UP regressions have explained tradeoffs, and agreed budgets pass. Unmet budgets
leave the stage open with the observed bottleneck. AP boot, average FPS or one
successful run do not close this roadmap.

## Order and verification

Critical path: `OP0 → OP1 → OP2 → OP3 → OP4 → OP5 → OP9`. Isolated AP boot/park
may follow CPU-local entry/audit without entering shared process/VFS/heap paths.
Complete ownership and remote-memory safety are required before OP4. Serial
OP6/OP7/OP8 optimizations may follow OP0 profiles; parallel SDK workers additionally
need OP4 and thread/sync. OP9 includes the mandatory scenes from OP6–OP8.

Keep commits independently meaningful and verified. Serialize shared builds/VMs;
start with host checks and exact-Makefile kernel compilation, then a narrow native
scenario through `tools/qemu_test.py`, broadening only for unresolved concerns.
Reuse compatible images. Set CPU count explicitly with `--cpus`; do not combine
it with raw `-smp`. Record commit, command, raw results, assertions and limitations
for each completed item. ABI, public stats, GUI wire, scheduling policy and other
system-contract changes follow the [contributor approval boundaries](.github/contributors/CLAUDE.md).
