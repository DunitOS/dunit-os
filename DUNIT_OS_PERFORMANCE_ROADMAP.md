# Dunit OS performance and SMP roadmap

Updated 15 November 2026. OP0–OP3 are partial; the next stage is to finish OP3
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
| OP3 | In progress | Console/stdin, remote stop/cancel, IRQ progress, lock ranking, concurrent VM stress |
| OP4 | Prepared only | AP application dispatch, remote wakeups, migration and overlapping execution |
| OP5 | Not started | Scheduler scaling, accounting and latency |
| OP6–OP8 | Open | New profile-driven input, rendering, memory and I/O work |
| OP9 | Open | Final correctness/scaling/performance and regression matrix |

A completed subtask below is a narrow implementation with evidence. It does
not imply acceptance of its whole stage.

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

Delivered: GUI wheel/short-field/long-field/selection-drag and wheel-burst
benchmarks; equal-work CPU/pthread workers on 1/2/4/8, allocator and MemFS I/O
with data/checksum checks; metadata/JSON provenance; distinct PIT, host-wall and
unmeasured metrics; checked queue/dispatch before-after series; native two-process
IPC ping-pong, warmup plus five runs of 1,000 checked round trips.

Remaining: idle/startup, loaded/multi-client and fixed game scenes; KVM reference
machine, noise and budgets; profiler overhead, CPU accounting, per-event latency
tails, queue/wait telemetry and RSS. Existing pointer/GUI correctness tests are
not full performance baselines.

Acceptance: one harness command reproduces equal data/pixels/counters, raw
repeats and noise. A TCG baseline is usable while KVM remains unavailable;
the KVM performance gate stays open.

## OP1 — shared-state audit and CPU-local entry

Delivered: shared-state/lock/lifetime inventory; CPU-local PID, syscall scratch,
stacks/escape pointers, scheduling flags and FPU pointer; private GDT/TSS/IST/AP
stacks; FS/TLS, SWAPGS, syscall/IRQ/exception/return checks; CPU-bound guards and
correct guard `Sync` bound; non-escaping public/private task access and deferred
fd notifications; copied HAL context and owned dispatch resources; registry/task
mutexes, identity revalidation and task pins with retirement outside guards.

Remaining: complete remote execution/cancellation and IRQ/preemption nesting/
idle model. A task pin keeps storage alive; it neither grants Running ownership
nor permits execution of a retired frame.

Acceptance: explicit owners/invariants, green UP regressions and no shared
writable entry scratch/stacks, then closure of all ownership gates before AP
application execution. An audit alone does not fix its listed races.

## OP2 — AP boot, APIC, IPIs and clocks

Delivered: Limine MP handshake and AP-local GS/GDT/TSS/IST/IDT/syscall MSRs and
x87/SSE setup; logical CPU indices independent of APIC IDs; xAPIC/IPI round
trips, EOI/spurious handling, BSP PIC routing and PIT-calibrated local timer
probes; BIOS 1/2/4/8 and UEFI four-vCPU TCG checks. APs park without perpetual
timer wakeups after probes. Global HPET validates ACPI/checksum/64-bit width and
progress, including IRQ-off and real AP brackets; PIT fallback and separate
clock/event metadata are verified without assuming synchronized cross-CPU TSC.

Remaining: precise timer-event/wakeup model, x2APIC, KVM and hardware coverage.
Acceptance: AP boot/park and work/timer/IPI checks on each supported platform,
with clock/event limits stated. This stage does not establish an SMP scheduler.

## OP3 — memory, ownership and synchronization

Delivered foundations:

- VFS operation-local paths, both rename endpoints, owning locked backends,
  scoped VFS access and final-close lifetimes; raw backend pointers removed.
- Detached fd/VM/capability cleanup batches, fd inheritance rollback and owning
  snapshots. Kill closes fd/pipe resources before wait; releases occur outside
  lifecycle mutation.
- Queue→task-table lock edge removed; generation-safe validation/requeue and
  bounded allocation-free dispatch traversal. Current/explicit-PID access and
  fd rescans use non-escaping scopes, including errors.
- Arc-owned stack/FPU/locked address space, copied HAL input, active-root pins,
  canonical kernel-half refresh and caller PID/CR3/stack restoration on errors.
- Wake callbacks outside wait-queue locks; generation cutoff, scheduler admission
  after task borrow and futex lookup/mapping validation before queue acquisition.
  Futex compare/register stays atomic; clear-child-tid uses Release.
- PMM leaf-lock transactions for allocation/contiguous/free/accounting, one boot
  bitmap handoff, host mixed-transaction stress and BSP/AP probes on 1/4/8 CPUs.
- Registry/task locks and pins across lookup, TLS/join/reap/kill, preemption and
  futex, with post-lock identity validation and no mutex held through HAL/IRET.
- Notification checkpoints and park identity for IPC/pipe/poll/wait-set producer
  gaps; delayed wake cannot affect a later park. Locked IPC FIFO, notifications
  after guards and atomic handle insertion/message publication.
- Acknowledged TLB invalidation: active roots, generation/ack, lock-free NMI
  receiver, user unmap/protect and kernel alias reuse. Address-space mapping pins
  protect validation through copying; atomic PTE operations avoid mutable whole
  page-table references. Isolated AP retarget/reentry/reclaim and dropped-ack
  refusal are tested.
- Locked PTY table, owning endpoint snapshots for I/O/ioctl/readiness, attachment
  identity across final-drop/reattach, notifications and last endpoint/file
  owners retired after task scopes on close/dup/install/bind/error paths.

Remaining gates, in order:

1. Console/stdin: transact owner and line state together; short console/scrollback
   scopes, no console lock through shell/exec; coordinate framebuffer render/
   modeset and remaining driver/heap edges.
2. Remote cancellation/stop acknowledgements and group finalization. Publish
   waitable exit only after executors stop and cleanup completes; cover nested
   dispatch, join/detach and cancelling waits. Check/register identity alone
   does not stop a running CPU.
3. Complete lock ranking/reentry for registry/task, VFS/ProcFS, waits, IPC/handles,
   heap/PMM/VMM and devices. Never sleep under a spinlock or wait for an ack while
   holding a lock the receiver needs. Establish interruptible syscall boundaries,
   bounded IRQ-off sections, IPI progress and remote runqueue notifications.
4. Prove heap/PMM/shared-frame/handle/device order and final-reader lifetime.
   Address-space/task/execution pins are different contracts; no one pin replaces
   mapping ownership or shared-frame bookkeeping.
5. Real remote CPL3/shared-VM/W^X fault, concurrent exit, migration and page-reclaim
   stress, with fault injection. Existing isolated AP probes and UP regressions
   do not replace this matrix.

Acceptance: no stale mappings, double frees, deadlocks or capability leaks.
General AP userspace dispatch is prohibited until these gates pass.

## OP4 — concurrent process/thread scheduling

Delivered preparation: exclusive execution reservations identify CPU/generation/
task, enter Running before HAL and disallow readmission until return/cleanup
handoff finishes. Retirement checks include owner/thread groups. Actual-lease
host stress and native/UP regressions pass. Remote cancellation and concurrent
applications remain open.

Implement AP application dispatch only after OP3; retain the exclusive
`Ready → Running(cpu)` claim. Add remote wakeup/reschedule IPIs, race-safe
block/wake/deadlines, CPU-local idle, and GPR/FPU/TLS preservation during
preemption/migration. A first correct scheduler may use one ready queue, but a
kernel-wide lock serializing all syscalls/runtime is not the performance goal.

Acceptance: workers/pthreads overlap on different CPUs and produce checked useful
results. On 1/2/4/8 CPUs stress shared memory, mutex/condvar/futex, IPC,
join/detach, kill/exit and FPU/TLS. Show KVM scaling against one CPU separately.

## OP5 — scheduler scaling and predictable latency

Use profiles to decide per-CPU ready queues, remote enqueue and bounded balancing;
measure contention, migration and cache locality. Add CPU accounting and
wakeup→run latency; shorten non-preemptible/IRQ-off sections. Introduce
interactive policy without starving background jobs, then idle/deadline/tickless
work after timers are correct.

Acceptance: loaded GUI meets OP0 budgets, no starvation/lost wakeups or busy idle,
and scaling improves or evidence shows the shared queue is not yet the limit.
Extra timer interrupts alone are not an optimization.

## OP6 — input, IPC and SDK latency

Measure kernel queue→scheduling→IPC→client update/render→commit/composition,
queue depth and burst recovery. Remove unnecessary wakes/copies/syscalls and
batch bounded updates while preserving ordering, every key/button transition,
wheel detents, LOST_BEFORE recovery, cursor modes and transfers. If synchronous
FrameDone limits throughput, specify bounded frames in flight/backpressure and
resize cancellation; published buffers remain immutable until their last reader
releases them.

Acceptance: p95/p99 and backlog improve on UP and SMP while exact text, selection,
undo, transfers, cursor lock, resize and frame completion remain correct.
Public protocol changes require an approved contract first.

## OP7 — software rendering and parallel SDK work

Profile damage/occlusion, layout/diff, glyph/text caches, SVG/icons, ARGB/blur and
memory bandwidth. Remove repeated layout/allocations/repaints only with proven
pixel equivalence and bounded caches. Compare release profiles/LTO/codegen for
speed, image size and reproducibility within the image/ESP budget. SIMD/blits
require feature checks and scalar fallback; AVX first requires CPUID/OSXSAVE
and scheduler extended-state preservation.

After OP4 and owning SDK thread/sync APIs, add bounded workers for independent
jobs/tiles. UI state, inbox and publication stay with their owner; use immutable
snapshots, correct dirty regions/blur halos and blending order. Measure game
simulation/rasterization separately from compositor cost.

Acceptance: scalar/optimized and serial/parallel paths agree on pixels, memory
stays bounded and fixed scenes improve. Keep small jobs serial when coordination
cost exceeds useful work.

## OP8 — memory, files and I/O

Measure allocator contention/fragmentation, cache misses/false sharing, shared
buffer/IPC copies and RSS. Apply bounded batching/locality/per-CPU caches with
reclaim only to observed limits. Optimize VFS lookup, buffering and block I/O
without changing coherence, rights, partial I/O, errors, durability or disk
format merely for a benchmark. Large copies/syscalls/drivers must not hold a
global lock or disable IRQs for their whole workload.

Acceptance: fixed-work throughput/latency/memory improve; fsck/recovery,
persistence/reboot and memory-pressure regressions remain green.

## OP9 — final matrix and regression protection

Compare 1/2/4/8 CPUs, KVM/TCG separately, cold/warm, idle/loaded, normal/heavier
supported resolution at identical effects/scenes. Short correctness/smoke gates
belong in CI; long stress/performance runs are separate. Enable numeric regression
thresholds only on a stable runner with OP0 noise evidence.

Publish before/after latency, frame distributions, scaling, throughput,
lock/IRQ-off hotspots, RSS and limits. If software rendering remains the limit,
propose GPU/virtio-gpu separately with a software fallback.

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
