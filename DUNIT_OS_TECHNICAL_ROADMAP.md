# Dunit OS technical roadmap

Updated 17 November 2026. This is the canonical architecture, dependency and
acceptance plan. Implemented foundations do not close an entire milestone:
M4 persistence, M5 installed root, M6 release gates and M7 SDK completion remain
open. [Current state](docs/architecture/current-state.md) describes the implementation;
[performance and SMP](DUNIT_OS_PERFORMANCE_ROADMAP.md) tracks OP0–OP9 separately.
Historical reports are evidence for their recorded commits, not fresh verification
of today's tree. Git history retains the superseded roadmap and vault.

`[x]` marks completed work with recorded verification. `[ ]` marks remaining
work or acceptance. Partial milestones contain both; a checked foundation does
not close its unchecked release gates.

## Purpose and architecture

Dunit is an independent x86_64 desktop OS: Green Tea Kernel, native Dunit ABI,
userspace services/apps, DunitFS and a configurable DWM desktop. It is not a Linux
ABI implementation. POSIX and the musl fork provide C source portability.

Limine hands BIOS/UEFI boot to the C/NASM HAL and Rust `no_std` kernel.
HAL owns hardware entry, GDT/TSS/IDT, syscall and context-switch assembly.
The kernel owns memory/VM, process/thread scheduling, IPC, capability objects,
VFS/block storage and device mechanisms. `init` supervises userspace services.
Independent Rust/C ELF processes use `libdunit`, the native SDK or Dunit musl.
`gui_server` owns display/input master rights and software composition; the
accepted M7.9 design keeps DWM policy inside that userspace process on shared DSS
styles. A future external policy client requires layers/struts/policy and recovery
contracts first. The reference E1000 driver feeds bounded packet queues to
userspace `netd`. Browsers will use a separate sandboxed network service.

Desktop apps, layout, themes, icons, workspace rules and widgets belong in
userspace/configuration. The kernel does not contain the removed desktop loop
or application-specific policy. Apps/assets load from external `DUNITRD1` initrd;
boot splash is the remaining kernel bitmap. System/data programs must eventually
load from the installed disk rather than treating initrd-backed MemFS as a
finished installed root.

The early vision's egui/relibc/smoltcp/NetSurf choices and fixed memory maps were
superseded by the native runtime, musl fork, `netd` architecture and deferred
browser-engine decision. Its enduring requirements are hardware-layer separation,
isolated ring-3 Rust/C apps, validated ELF/syscalls, memory protection, IPC/shared
buffers, userspace composition, device interfaces and source portability.
The current kernel includes drivers/VFS; it is not a completed microkernel with
all drivers moved to userspace.

## Current foundations and explicit gaps

| Area | Implemented foundation | Open limits |
| --- | --- | --- |
| Boot | Limine BIOS/UEFI, deterministic Terminal/GUI configs, ISO/disk images, real initrd | Normal disk-backed root and complete first-boot profiles |
| Memory | PMM/VMM/growable heap, isolated ELF, safe user copy, anonymous/private file mmap/unmap/protect, W^X/guards/shared objects | Shared file mappings, demand paging/COW, dynamic loader/TLS, swap |
| Runtime | Preemptive UP, schedulable threads, FXSAVE x87/MMX/SSE, static TLS, futex, wait queues, clocks | Concurrent AP applications, priorities/accounting, AVX/XSAVE, signals/interruption, process-image exec/fork |
| Objects/ABI | Generated native ABI, bounded IPC with sender identity, narrowing-only handles, display/input exclusivity, process-owned Ethernet queues | Public release compatibility suite, edge/persistent wait-set, application sockets |
| Files | VFS/MemFS root, fd/cwd/file APIs, dup/pipe/fcntl/poll/ioctl and atomic spawn actions | Independent complete DevFS/ProcFS contracts, disk root, ownership/groups/permissions and links |
| Storage | AHCI DMA, legacy VirtIO polling, GPT policy/CRC, transactional installer, DunitFS v2/fsck/degraded mount | 64-node limit, 1,000-file acceptance, broader recovery/hardware and user provisioning |
| Desktop | Userspace compositor/DWM, TOML/DSS/Lua, shared themes, SDK GUI apps, clipboard/DnD, pointer modes | Session geometry across reboot, declarative whole-window effects, frame budget, IME/a11y/shaping |
| C | Static Dunit musl, native pthread subset, TinyCC on-device compile/link/run, thin generated C UI v1 | Secure entropy, broad POSIX/release gates, dynamic tier, in-memory TCC `-run` policy |
| Hardware | PS/2, PCI, framebuffer, AHCI/VirtIO, reference E1000 INTx, reboot/poweroff, AP boot/park and HPET/IPIs | Broader NIC qualification, complete USB/audio, modern VirtIO, ACPI battery/thermal/suspend |

AP boot/probes work up to eight vCPUs in the recorded TCG matrix; application
execution remains `dispatch_cpus=1`. HPET clock progress and PIT fallback are
validated, while timer events remain PIT 100 Hz. Realtime uses a valid Limine UTC
boot timestamp plus monotonic elapsed time; absent valid boot UTC it returns an
error. This is neither an RTC driver nor secure entropy. `AT_RANDOM` still uses
a bootstrap TSC seed and cannot support cryptographic/TLS claims.

## Contracts and verification

`abi/*.abi` generates Rust/C numbers, errno and rights. Preserve native register
calling convention, LP64 public types, endian/alignment and versioned wire
structures rather than exposing kernel-private or Linux structures. Specify fd/
handle lifetime/inheritance and reserved 0/1/2; process stack/argc/argv/envp/auxv,
page size/random seed/program headers; accepted ELF types/segments/load bias,
stack permissions, PT_TLS/RELRO/INTERP policy and feature queries. Human approval
is required for unapproved ABI/wire/rights/disk-format or system-policy changes;
see [contributor rules](.github/contributors/CLAUDE.md).

All OS builds/boots/tests use `tools/qemu_test.py`; host model tests complement
native checks. Keep executable claims and source-linked required/forbidden
markers, warning budgets and artifact gates. A screenshot accompanies state/
serial assertions. Persistence requires complete VM stop/start, remount and
content/config hashes. Mark expected fault/unsupported cases explicitly; no
unexpected panic, FAIL, timeout or leaks. Unsupported API returns documented
ENOSYS/ENOTSUP rather than fake success.

The retained acceptance matrix covers ISO/disk × Terminal/GUI × BIOS/UEFI;
AHCI/VirtIO × clean/corrupt/full/power-cut; spawn/wait/kill/fault/preempt/thread/
TLS; IPC saturation/crash/close/rights/shared buffers; headless GUI conformance
plus native screenshots/input; static libc tiers; future isolated network and
browser-security tests. Preserve 10-minute smoke and multi-hour nightly stress
as longevity goals. Builds/QEMU sharing artifacts and Git mutations are serialized.

## M0 — truthful baseline and executable contracts

- [x] Maintain an English current-state snapshot and native ABI documentation.
- [x] Replace misleading boot claims with observable readiness/failure markers.
- [x] Enforce CI warning budgets and source-linked required/forbidden markers.
- [x] Verify the Terminal/GUI and BIOS/UEFI baseline through the shared harness.
- [x] Remove the unused Rust bootloader experiment; Limine remains the bootloader.
- [ ] Keep every working claim linked to its source/test and dated evidence.
- [ ] Version future process-entry, handle, GUI, libc and service decisions.
- [ ] Preserve repeatable baseline checks and lower warning budgets as debt clears;
  source-file existence is never acceptance evidence.

## M1 — kernel runtime prerequisites

- [x] Enable UP timer preemption, checked with CPU-bound parent/child without yield.
- [x] Give threads separate GPR, kernel stacks and FXSAVE state, sharing process
  address space and fd ownership.
- [x] Implement thread create/exit/join and isolate faults to the affected thread.
- [x] Block on sleep, IPC and timeouts without a userspace polling loop.
- [x] Implement anonymous VM partial unmap/protect, guards, W^X and shared-frame
  teardown; eager private file mappings retain their documented EOF limits.
- [x] Initialize independent Variant II static PT_TLS and FS.base per thread.
- [x] Implement atomic compare-and-park futex wait/wake with timeouts.
- [x] Provide capability memory/endpoints/display/input and narrowing-only transfer.
- [ ] Complete owned/locked/atomic global-state migration and SMP safety in OP1–OP3.

Depends on M0. Acceptance: preemption/starvation stress, distinct XMM/TLS/errno,
wait/wake races without lost wakes or busy waiting, mapping boundaries and kill/
exit resource cleanup. IRQ/lock inversion, FPU leakage and use-after-unmap must
be tested. Dynamic TLS/DTV, AVX, accounting/priorities and SMP are separate gates.

## M2 — GUI protocol and headless reference models

- [x] Publish the versioned gui-v1 binary specification and common codec.
- [x] Test framing/negotiation/errors, serials/namespaces and surface/buffer/
  configure/ack/commit lifetimes with deterministic headless models.
- [x] Test frame callbacks, focus/input/grabs, geometry and quotas.
- [x] Keep seeded malformed-byte/event tests and useful pure window/composition models.
- [x] Remove the obsolete egui process lifecycle.
- [ ] Freeze a public release contract only after its separate compatibility gate.

GUI contracts negotiate features and bound little-endian records and damage.
Shared XRGB8888/ARGB8888 buffers have explicit size/stride/offset/ownership;
clients receive capabilities, never the physical framebuffer. Ordinary and
privileged operations stay distinct. Validate framing/attachments, versions,
serials, directions/policy, state/namespace and geometry/quotas without partial
state mutation. Committed-scene hit testing, focus enforcement and frame
callbacks replace busy immediate drawing. GPU is a later backend.

Can be designed alongside M1; native integration depends on it. Acceptance:
malformed/stale/cross-client objects cannot damage the server, buffer bounds and
reuse are checked, focus/input ordering and replay are deterministic, fuzzing
cannot panic. Public release freeze remains a separate ABI gate.

## M3 — userspace compositor

- [x] Provide exclusive display/input authority and shared-buffer mechanisms.
- [x] Run gui_server as an ordinary ELF process with two isolated clients.
- [x] Implement software composition, frame lifecycle and focus/input routing.
- [x] Authenticate IPC senders in the kernel and use the common protocol codec.
- [x] Remove the kernel GUI, GUI1 and dead display-server crates.
- [ ] Complete the longer idle/load, resize-storm and GUI-service recovery matrix.

Depends on M1/M2. Acceptance: overlapping clients, resize storms, reuse, input
isolation, crash cleanup and 10-minute idle/load. Malformed clients disconnect,
stale objects fail explicitly and one app crash removes only its surfaces.
GUI-service failure must leave the kernel/recovery console usable; supervision/
restart/fallback is part of service integration. Do not rewrite composition in
the application widget layer.

## M4 — configurable DWM and UI runtime parity

- [x] Provide independent runtime crates for TTF text, DUI retained/two-pass layout,
  DSS cascade/tokens/states/motion, widgets and software painting.
- [x] Implement interactive windows, panel/dock/launcher/tray, quick settings,
  notifications, workspaces and the switcher.
- [x] Resize from every edge/corner with app-specific minimum content sizes;
  verify raise, move, maximize/restore, minimize, close/reopen and keyboard input
  through real pointer/key events and captured frames. Calculator's complete
  keypad remains usable at its configured minimum.
- [x] Show live processes in tray/monitor counts, including blocked tasks;
  verify changes across application launch, minimize, close and reopen.
- [x] Honor disabled session restore for loading, remembering and saving geometry;
  keep the saved store when toggling the policy off.
- [x] Ship Terminal, Files, Calculator, Monitor and Settings; run userspace dsh over PTY.
- [x] Share validated configuration between setup and Settings.
- [x] Run native Lua/luac and isolated policy/event workers with deadlines and
  working-geometry fallback. TOML owns policy, DSS appearance and Lua behavior.
- [x] Load app/widget manifests, fonts, wallpapers and five Chameleon icon themes
  externally, with shared static SVG/bitmap scale, aliases and fallback.
- [x] Verify pointer geometry and display mode switching through 3072×1920.
- [ ] Expand SVG features only with their own conformance; the static profile is bounded.

Use stable UI IDs and Row/Column/Stack/Grid/Scroll constraints, common widgets,
capture/bubble events, keyboard focus, cascade defaults→theme→class→state and
native property interpolation. TOML/DUI/DSS schemas need independent versions,
size/depth limits and offline linting. Reload is parse→validate→resolve→dry
layout→atomic swap; retain last-known-good on failure. Configuration must change
runtime behavior, not just parse successfully. Filesystem watching remains a
future event facility; explicit reload exists.

- [ ] Prove session/window geometry persistence across reboot, beyond reload/reconnect.
- [ ] Finish persistent user profiles after M5 installed-root work.
- [ ] Provide filesystem change events; explicit reload already exists.
- [ ] Complete accessibility, shaping/IME and whole-scene frame budgets.

Acceptance: full feature parity, keyboard-only use, malformed reload rollback,
1024×768/1600×900/HiDPI visual/layout tests and animation timing; persistent
configuration/session hashes across stop/start. Appearance changes require no
kernel rebuild. Brightness UI eventually uses display/power service control.

## M5 — installed root, provisioning and recoverable storage

- [x] Validate GPT ESP/Dunit System/Data policy, excluding foreign partition types.
- [x] Supply real initrd/service manifests, supervision/restart budgets and
  `/persist/.provisioned` state.
- [x] Implement DunitFS v2 bitmap/extents, directories, rename/unlink, timestamps/
  mode, fsync, generations/metadata COW, fsck and degraded read-only mount.
- [x] Run transactional validate→partition→format→copy→verify→boot-config→sync
  installation with rollback.
- [x] Support AHCI/VirtIO installation paths and BIOS/UEFI profile/image manifests.
- [x] Honor VirtIO read-only disks in device metadata, block writes and installer
  validation; rejected writes keep reads available and leave the image unchanged.
- [ ] Preserve partition/device identity, bounds and explicit authorization on real disks.

- [ ] Load root, init, services, apps and assets from the installed system volume.
- [ ] Enforce a read-mostly system volume and complete system/user provisioning.
- [ ] Implement ownership/groups/permissions, links and orphan cleanup.
- [ ] Scale metadata beyond 64 nodes and pass the 1,000-file acceptance scene.
- [ ] Complete interruption/recovery coverage. Initrd-backed MemFS and writable
  /persist do not close installed-root acceptance.

Target hierarchy: `/system/{bin,lib,share,services}`, `/apps/<id>`,
`/users/<uid>/{home,config,data}`, `/var/{log,lib}`, volatile `/run`, device
namespace `/devices` and diagnostics `/proc`. Current `/app`/`/assets` come from
initrd, `/tmp` is volatile and `/persist` holds durable data/config; migrate these
through an explicit installed profile rather than pretending it already exists.
DunitFS needs version/feature/backup-generation contracts and bounded recovery;
full POSIX, snapshots, network/distributed FS and ext2/FAT root substitution are
outside the first storage milestone.

Acceptance: two-reboot content/config hashes, directory/rename/unlink persistence,
1,000 files/large files, empty/sparse-file policy, disk-full and corrupt CRC;
metadata power-cut points recover old or new state, never a mixture. BIOS/UEFI ×
AHCI/VirtIO disk boot works without ISO or installer modules, with explicit
recovery/read-only behavior on damaged volumes. Kernel contains no app binaries.
Base storage can proceed alongside M1–M3; M4 persistence and M6 disk tools depend
on it. DunitFS v1 must not be the sole copy of valuable data.

## M6 — native static musl and C toolchain

Static core implemented; release/broader profiles remain open.
The complete pinned `dunit-musl` fork is primary; relibc/newlib are comparison
fallbacks only if a technical spike finds a fundamental blocker. Preserve
upstream provenance/licenses, controlled generated delta, unsupported source
subsystems and regular bug/security updates. Concentrate changes in crt/native
syscall/error/structure adapters, VM/fs/process/thread backends; retain generic
CPU/ISO C code and avoid scattered Dunit conditionals or upstream reformatting.
Kernel never consumes Linux numbers/structs or public musl structs.

- [x] Build logical x86_64-dunit through the static compiler-driver/LLD flow.
- [x] Stage crt1/crti/crtn, public/native headers and libc.a.
- [x] Provide compiler/ar/strip wrappers and SDK version/license metadata.
- [x] Reject host headers/libraries and generate CMake/Meson/Autoconf cross files.
- [x] Invalidate stale sysroots through source/compiler fingerprints.
- [x] Record an identical clean/offline libc archive rebuild.
- [ ] Rehearse upstream upgrades and publish a versioned release capability profile.
  Full GCC or Rust-on-device ports are not prerequisites.

Profiles and remaining gates:

- [x] Profile A: crt/init/main/exit, fd 0/1/2, stdio, strings/math/conversion,
  constructors, arguments, environment and auxv.
- [ ] Maintain Profile A startup/stack/BSS/error/invalid-pointer conformance.
- [x] Profile B: mmap/unmap/protect, allocator without Linux brk, zero-fill/alignment,
  monotonic time and valid-boot-UTC realtime.
- [ ] Supply secure entropy and extend fragmentation/OOM/protection/clock-edge coverage.
- [x] Profile C: buffered FILE, open/read/write/seek/stat/cwd/directories/rename/
  unlink/fsync/truncate and strict flag/error adapters.
- [ ] Run libc/tools from installed root and complete ownership/accounts/storage matrix.
- [x] Profile D: native posix_spawn argv/env/cwd, pipes/dup/redirection/fcntl/poll/
  ioctl and atomic ordered open/close/dup2/chdir actions.
- [ ] Implement process-image exec/fork only after their contracts; reject unsupported
  spawn attributes until supported.
- [x] Profile E: native thread/TLS/futex, create/join/detach, errno, mutex/condvar/
  rwlock/once/barrier and heavy-resource reclamation.
- [ ] Define owner-death, cancellation/interruption and broader thread-primitives policy.
- [x] Profile F: enforce the official static-only build policy.
- [ ] Add signals/timers/interruption, sockets after netd, dynamic loader/TLS and
  demand-driven fork/COW as independently gated work.

`pthread_cancel`/asynchronous cancellation explicitly fail ENOTSUP; system/popen
and fork-dependent paths cannot silently succeed. Define signal delivery frames,
masks, restart/interruption, alternate stacks and thread targeting before enabling
cancellation points. Dynamic tier needs PT_INTERP, DSO mappings/relocations/symbol
lookup, RELRO, TLS modules/DTV and dlopen/dlsym/dlclose. Fork requires COW,
multithreaded semantics and atfork only when justified by software demand.

Acceptance: host-independent static ELF hello/args/env/files/malloc/spawn/pthread
suites on Dunit, 1,000 join and detached cycles, contention/timeouts/TLS/preemption
and threaded allocator stress; static ELF gates reject executable stacks/W+X,
unexpected DYNAMIC/INTERP/unresolved symbols. Persistence checks content after
power cycle. Retain pinned upstream libc-test subset and full unsupported-case
inventory with reasons/owners; subset success is not broad POSIX conformance.

- [ ] Verify previous-release binaries and freeze ABI/libc release contracts.

Network and dynamic suites stay separate from static/thread/persistence gates.

The recorded M6 repair passed 40 contracts/73 commands/175 required markers with
no forbidden markers; the prior pipe regression was fixed. Its 128 KiB persistent
hash and thread reclamation establish those slices, not secure randomness,
installed root or a complete POSIX release. See [M6 readiness](docs/architecture/current-state.md#historical-validation).

## M7 — native application SDK

Partial overall. The approved runtime is `no_std + alloc` over `libdunit`;
full Rust std is a separate future port. Hosts cross-build applications; on-device
Rust compilation requires its own rustc/toolchain work. Low-level ABI/kernel/C
fixtures stay on libdunit. Services (`gui_server`, `init`) consume lower shared
runtime/protocol APIs. Applications migrate fully to SDK, retaining functionality
and removing duplicate handshakes, unsafe entry and manual standard controls.

The 17 November thread/sync slice (`c08a900`) and initial private E1000
bring-up (`80c53ef`) have [integrated verification receipts](docs/verification/multitrack-2026-11-17.json).
The [follow-up slice](docs/verification/multitrack-progress-2026-11-17.json) adds
standalone SDK packaging/offline relocation checks, interruptible syscalls and
private external INTx exchange. Neither slice closes the remaining M7 or M8
release gates.

### M7.1 — wait-set

- [x] Implement WaitSetWait (89) for call-scoped level-triggered IPC, timer/deadline,
  file/pipe/PTY and child-exit readiness, with safe wrapper and owning event loop.
- [x] Preserve tokens, timeout/fd identity/deadlines and cancellation via IPC.
- [x] Keep previous IPC/futex/wait/poll calls compatible.
- [ ] Add edge/persistent registration and broader integration.
- [ ] Add socket readiness after M8.

Acceptance: pipe/IPC/timer wake returns the correct token, close produces hangup,
timeouts/lost-wake races pass and real event loops block without polling.
Depends on M1; future contract expansion needs approval.

### M7.2 and M7.3 — runtime, tooling and app manifest

- [x] Provide CLI/TUI/GUI cargo dunit new/build/run/test through the shared harness.
- [x] Supply native #[dunit::main] argv/env/panic/exit and compiler-collected tests;
  empty suites, Err and panic fail explicitly.
- [x] Provide owning fs/process/time/io/env/args/stats and shared Error/errno APIs.
- [x] Validate dunit.toml v1 identity/name/icon/kind, exact Cargo binary target and
  unique requested capabilities; declarations grant no rights.
- [x] Feed launcher/dock/startup through packaging and a shared userspace catalog.
- [x] Launch CLI/TUI with configured Terminal and separate argv/PTY, without shell
  interpolation.
- [x] Implement deterministic standalone SDK packaging, safe verified installation,
  offline CLI/TUI/GUI builds and relocation with projects; retain one delegated
  OS harness for native run/test. See [distribution contract](docs/sdk/standalone.md).
- [x] Verify standalone native CLI/C execution and GUI run/200-frame tests through
  the delegated harness, including the unchanged 32 MiB memory budget.
- [ ] Finish remaining legacy-catalog migration.
- [x] Provide owning SDK threads, joining handles, futex mutexes and condition variables
  over the existing ABI; see [thread/sync limits](docs/sdk/threads.md).
- [ ] Provide the future dunit::net backend.

Acceptance: generated apps have no unsafe/extern-C/raw entry and cross-build,
package and run with checked output/window;
`cargo dunit test` supplies its own markers. At least one migrated real app uses
this route. Capability declaration, cwd/env, spaces in paths, CLI/TUI input,
child cancel/exit and cleanup must retain native checks.

### M7.4 and M7.5 — typed UI, shared styles and effects

- [x] Provide owning Component/update/view with retained stable control keys,
  layout/style/tree diff, old/new damage repair and clipped repaint.
- [x] Reuse shared buffers, skip no-op frames and own configure/resize/input/frame/
  close lifecycle through Window/ComponentWindow.
- [x] Propagate real texture/composition/present damage for stable opaque/unscaled/
  unobscured scenes; use full composition for overlap/effects/scene changes.
- [x] Keep the existing gui-v1 wire format.
- [x] Share typed DSS colors/fonts/spacing/radii/borders/gradients/shadows/clips and
  client-canvas backdrop blur, preserving model and last-good light/dark appearance.
- [x] Sample, retarget and reduce stable-key timed transitions through wait-set.
- [ ] Complete nested component lifecycle and geometry/full-widget animations.
- [ ] Provide declarative whole-window transparency/blur/rounding without manual ARGB.
- [ ] Expand adaptive assets/manifests, visual references and static SVG features.
- [ ] Add complex text shaping/IME as later work.

Acceptance: message sequences prove model/tree state; incremental/full pixels
match, old shadows/blur sample regions repaint correctly, tiny changes bound
composed pixels and no-op sends no frame. Default apps in both themes pass pinned
font/scale goldens with no private appearance literals; theme change repaints all
token areas. Whole-scene transparent-window ≤16 ms remains an unachieved target,
tracked with reproducible performance evidence rather than average FPS.

### M7.6 — widget behavior

- [x] Implement push/toggle/check/radio/switch controls.
- [x] Implement controlled Input/Textarea, grapheme/word/line selection, soft wrap,
  undo/redo, password/readonly and limits.
- [x] Share two-axis scrolling, selection autoscroll and auto-hide behavior.
- [x] Provide horizontal/vertical sliders, integer spinboxes and progress/busy states.
- [x] Provide select, menu bar/submenus/context menu and tooltips.
- [x] Provide tabs/splitter/toolbar/status/dialog/modal/message box.
- [x] Virtualize list/tree/table with stable selection/expansion/sort/F2 rename.
- [x] Provide Open/Save/Directory file pickers.
- [x] Share state-aware flex/grid, capture/bubble stop/prevent, scoped mnemonics,
  roving focus, lifecycle cancellation and paint/hit geometry/styles.
- [x] Pass recorded event/model, native gallery, 10,000-row bounded-viewport and
  light/dark/restore exact-pixel widget acceptance.
- [ ] Define cross-surface popups/global grabs and inter-window modality.
- [ ] Provide accessibility/IME services; role labels alone do not establish them.

Each control specifies state transitions, pointer/keyboard/focus/resize and role/
label metadata. Negotiated key-up/focus snapshots, queued short pointer events,
modifier snapshots and Normal/Confined/Locked pointer modes preserve legacy
clients and release capture on focus/workspace/escape/fault. Window-relative raw
deltas remain distinct from screen-clamped coordinates.

Widget acceptance is recorded complete: event/model tests, real native gallery,
10,000-row virtualization bounded by viewport, light/dark/restore exact pixels.
System clipboard is M7.7; GUI migration is M7.11. Cross-surface popups/global grabs,
inter-window modality and IME require separate contracts; role labels do not
claim an accessibility/IME service. Widget-only tests cannot close all of M7.

### M7.7 and M7.8 — transfers and protocol ownership

- [x] Centralize CAP1/INP1/RLD1/NOT1, GUIC and generated XFR1 transfer values.
- [x] Deliver committed-buffer damage to CPU textures and framebuffer presentation;
  CI rejects duplicated control tags.
- [x] Broker typed text/files/images/custom-MIME clipboard/DnD in gui_server.
- [x] Negotiate copy/move/link, capture/feedback/highlighting and on-demand shared data.
- [x] Transfer owned/RAII attenuated file/directory capabilities to file targets and
  quoted paths to text targets, retaining authorization and rollback across IPC.
- [x] Remove Files' private clipboard.
- [x] Pass the recorded 49 ordered source/target pairs across seven production apps.
- [ ] Expand the matrix with each new catalog entry; completed transfer does not
  imply a revocation contract.

Acceptance recorded complete: all 49 ordered source/target pairs among seven
production GUI apps, with new catalog entries failing incomplete matrix coverage;
real file contents/shell output, multiple files/Unicode/quotes, workspace crossing,
new-process open, source/target crash, clipboard persistence after source exit,
Escape/unsupported without data delivery, readonly/password fields and DunitFS
sync/reboot. Unsupported file Link or cross-backend Move remains explicit and
must not be imitated by copy. No duplicate wire definitions or handshakes.

### M7.9 — compositor shell decision

- [x] Keep the accepted compositor-hosted shell in gui_server on shared DSS.
- [x] Share palette/metrics/cursor/shadow/control geometry and atomic candidate validation.
- [x] Save only intentional Settings overrides, preserving DSS inheritance,
  independent tables/comments and no-op files.
- [x] Exclude format arithmetic/resource limits appropriately from appearance gates.
- [ ] Meet the ≤16 ms whole-frame goal; recorded 21 ms markers do not meet it.
- [ ] Define external layers/struts/policy/restart contracts before shell extraction.

A normal external SDK-client probe established isolation/publication/boot/recovery,
not a complete shell replacement. External layers/struts/policy/restart contracts
are absent, so extraction is deferred. The compositor is not rebuilt on widgets.
Recorded 21 ms frame markers did not meet the ≤16 ms goal; input series include
slower runs and do not establish universal speedup/non-regression.

### M7.10 — thin generated C SDK

- [x] Provide dunit.h/libdunit_ui.a over the shared Rust widget/style/render runtime.
- [x] Generate C/Rust records/constants/callbacks/layouts/exports/signatures from
  runtime/dunit-c/api.json and the existing ABI/protocol owners.
- [x] Keep C model/update data separate from SDK input/focus/render ownership with
  bounded copied/borrowed lifetimes and error/reentry/reopen rules.
- [x] Expose core/checkable/radio/disabled, layout/styles/text/numbers/progress/menus,
  collections/workspace/file-picker and clipboard/DnD/file-capability APIs.
- [x] Protect additive C ABI v1 through snapshots and an old object surviving a
  forced Rust rebuild and guest link-only execution.
- [x] Verify native TCC compilation and exact Rust/C pixels through one toolkit.
- [ ] Keep image/ESP size and future C ABI changes within measured compatibility gates.

- [ ] Decide in-memory tcc -run/JIT policy independently. Existing RW→RX mprotect
  does not approve a new executable-memory privilege model.
- [x] Compile with file-based tcc -o and launch through native spawn.

Future C ABI changes follow version/compatibility gates; do not duplicate the
Rust toolkit.

### M7.11 — migration completion

- [x] Migrate calc, dufetch and Monitor to native APIs, and portable wc_demo to C headers.
- [x] Migrate all seven GUI clients to retained SDK controls and shared special canvas.
- [x] Remove obsolete entry/panic, GUI handshakes/buffers, standard controls, font
  loaders, raw PTY and protocol copies with before/after regressions.
- [x] Use runtime facts in dufetch rather than invented identity literals.
- [x] Migrate dtop to SDK entry/stream helpers and measured memory/process counters;
  report CPU usage unavailable and support a terminating --once snapshot.
- [ ] Define a versioned identity/uname source.
- [x] Migrate dsh and fsck_dunit to native SDK entry/streams/files/environment/
  diagnostics/process/PTY/time APIs; preserve cooked input and report exit codes.
- [x] Migrate init service I/O, filesystem, owned-child supervision and time to
  SDK lower layers; preserve its explicit entry and never-exit panic policy.
  Prepared-child recovery, restart budgets and single fallback reaping are checked.
- [ ] Finish remaining CLI/TUI/service lower-layer migration.
- [x] Migrate Setup to SDK entry and buffered I/O; reuse the desktop settings
  parser, staging rules and import paths.
- [x] Migrate BMP Viewer and Image Demo to owned SDK display leases. Submit
  each image as one bounded raster; compare the native BMP capture pixel for pixel.
- [x] Deliver owning SDK thread/synchronization APIs with guarded stacks, typed
  results and native join acknowledgement before reclamation.
- [x] Deliver standalone SDK packaging and verified offline relocated builds.
- [x] Verify native GUI frame/resource acceptance; prepare compositor resources
  before client bootstrap and publish the first callback after framebuffer present.
- [ ] Close final M7 acceptance with real cargo dunit apps and completed source-of-
  truth migration, without duplicate raw entry/GUI lifecycle/hit-test.

Preserve each program's existing outputs/markers when removing its implementation;
keep changing counters variable while validating their format. Migration must
improve usability while preserving behavior, not just rename wrappers.

M7 order: wait-set/runtime decisions → tooling/manifest → typed UI/themes/widgets
→ shared protocol → transfers → compositor styles/decision → C bindings → full
migration. M7 supports M8 wait-set/network/service APIs, M9 apps and M10 browser;
independent design can proceed, but releases cannot bypass these dependencies.

## M8 — networking and platform services

The reference QEMU E1000 has a kernel-owned DMA and INTx path with copied
packet queues for an authorized userspace service. The initial supervisor
grants the device only from an immutable boot catalog. The default netd service
opens the queue, acquires DHCPv4 unless the image supplies static IPv4, and
serves ARP/ICMP on the configured address. See
[native packet queue](docs/network/native-packet-queue.md) and
[private bring-up diagnostics](docs/network/e1000-bringup.md).

The kernel owns PCI/MMIO, DMA-safe memory, reset, IRQ and bounded RX/TX. Netd
owns protocol and interface policy. Ordinary applications need native service
sockets/resolver APIs and musl adapters. Shared CPU rings are a later optimization
after measuring the copied path. VirtIO-net follows reference transport
qualification. RTL8139 is later compatibility.

### N0 — E1000 packet I/O

- [x] Implement the private reference diagnostic: bounded DMA descriptors/buffers,
  wrap/payload/completion/reset/reclaim and production discovery without ambient
  bus mastering. PHY polling remains diagnostic only.
- [x] Add a private opt-in bounded INTx drain, CPU receive queue and external
  transformed-payload exchange through the shared harness. Require both guest
  success and 103 checked host responses; test ring wrap, reset and DMA reclaim.
  Native TCG checks cover 2/4/8 vCPUs; full N0 gates below stay open.
- [x] Add the approved copied packet ABI, boot-catalog grant, and process-owned
  non-inheritable E1000 queue with read/write/poll/wait-set and fail-closed stop.
  Route the reference service through that queue. Broader hardware qualification
  and application sockets remain open.

- [ ] Implement reset/register/MAC/link setup and a common DMA API.
- [ ] Own RX/TX descriptors, heads/tails, barriers and recycling.
- [ ] Support MSI/MSI-X with legacy fallback and bounded IRQ drain/moderation;
  polling is diagnostic only.
- [ ] Handle malformed/oversize frames, link changes, stalls/exhaustion/reset.
- [ ] Count packets/bytes/drop causes/checksums/full queues/resets/transitions.

Acceptance: bidirectional frame path,
ring wrap/starvation/completion/link-toggle/interrupt-storm/reset tests and
10-minute flood without DMA corruption/leaks; guards and ownership assertions.

### N1 — Ethernet, ARP, IPv4 and ICMP

- [x] Add a bounded userspace reference core for Ethernet II, ARP resolution,
  fixed-header IPv4 and ICMP echo, consumed by host tests and a native logical
  peer diagnostic. See [scope and limits](userspace/net-core/README.md);
  this does not close NIC transport, routing or production N1 acceptance.
- [x] Add bounded connected/default routes and longest-prefix next-hop selection
  to the reference core. Gateway ARP retains the final IP destination, interface
  MTU is enforced, and route errors precede neighbor mutation. Native logical
  diagnostics check the bytes; production interface/transport policy remains open.

- [ ] Validate Ethernet II sizes/padding/types; reject bad/unknown frames early.
- [ ] Bound ARP INCOMPLETE/REACHABLE/STALE retry/expiry/poisoning policy.
- [ ] Validate IPv4 version/IHL/length/checksum/TTL/protocol/address.
- [x] Install the connected route and configured gateway in stock netd.
  Use longest-prefix selection for validated DHCP classless gateway routes
  and verify off-link return traffic through E1000.
- [ ] Define MTU/TX fragmentation and bounded-or-deferred RX reassembly.
- [ ] Rate-limit ICMP echo/unreachable/time-exceeded and provide logical loopback.

Acceptance: ping gateway/host on ARP miss/hit/expiry,
bad lengths/checksums/TTL, route/ARP timeout, MTU/fuzz cases without service panic.

### N2 — UDP, DHCPv4 and DNS

- [x] Add UDP checksums, port demultiplexing, ephemeral allocation and bounded
  queues to the logical peer core. Exercise bind, connect, send, receive and
  readiness through the host and native diagnostic.
- [x] Add bounded DNS A/AAAA/CNAME parsing, compression, TTL caching and query
  validation/retry/timeouts over the logical UDP core.
- [x] Exercise native ARP, ICMP and UDP/DNS over the copied E1000 queue in QEMU.
  Verify wire bytes with the isolated peer before and after a supervised restart.
- [x] Expose UDP send-to and receive-from through explicitly granted
  nonblocking sockets. Verify readiness and truncation through E1000.
- [ ] Integrate POSIX blocking/timeouts and wider application socket policy.
- [x] Run the single-interface DHCPv4 state machine in netd. Validate offers,
  publish address/prefix/gateway/DNS together, renew, rebind, and remove expired,
  NAKed or link-invalidated configurations.
- [x] Prefer archive-provided static IPv4 with gateway/DNS settings over
  automatic DHCPv4. Probe before activation and withdraw on conflict.
- [x] Install up to seven DHCP option 121 routes via connected gateways or
  directly on-link. Prefer the classless list over Router option 3, preserve it
  across same-server renewals that omit route options, and exercise direct
  delivery and route withdrawal on E1000.
- [x] Validate DNS fallback over TCP after a matched truncated UDP reply.
  Exercise loss of SYN, split TCP response framing and close through E1000.
- [x] Cache DNS NXDOMAIN and NODATA using the authority SOA lifetime and
  the existing eight-entry cache. Verify type scope, TTL expiry and a
  cached negative result through E1000.
- [x] Bound DNS resolution while waiting for ARP. Retry on a secondary DNS
  endpoint after primary UDP timeout, route loss, SERVFAIL or REFUSED.
  Verify fresh transaction matching and failover over E1000.
- [x] Probe each new DHCPv4 address before activation and announce it after
  the quiet interval. Decline an ARP conflict and verify reacquisition after
  the cooldown through the E1000 fixture.
- [x] Accept DHCPv4 replies forwarded by a relay. Verify leases from an
  off-link server, routed unicast renewal and relayed NAK over E1000.
- [x] Send INIT-REBOOT for a remembered address supplied at boot. Require a
  fresh matching ACK and ARP conflict checks before activating it. Fall
  back to discovery on NAK or retry exhaustion.
- [x] Store a MAC-scoped DHCP address hint on DunitFS after activation.
  Revalidate it on the next boot and clear it on conflict, expiry or NAK.
  Verify both boots against the same persistent volume.
- [ ] Integrate the resolver with lease changes, service entropy, and an
  application endpoint. Define admin/user configuration precedence.

Acceptance: DHCP loss/NAK/
renew/duplicates, DNS NXDOMAIN/malformed/cache/timeout and UDP overflow.

### N3 — TCP

- [x] Implement the reference TCPv4 connection with active/passive and close
  states, four-tuple, sequence, ACK, flag, checksum and window validation.
- [x] Add bounded SYN/data retries, RTT/RTO, cumulative ACKs and zero-window
  probes. Test sequence wrap, half-close and timeout.
- [x] Keep TCP alive while the receiver acknowledges zero-window probes.
  Resume queued bytes on a window update and keep unanswered probes bounded.
- [x] Verify challenge ACK for an in-window, non-exact TCP reset and delivery
  of a valid reset to native Rust and C socket waiters. Retire the failed
  transport while the application descriptor can still report its error.
- [x] Bound individual send/receive queues. A gap is discarded and acknowledged
  cumulatively so recovery can retransmit it.
- [x] Add reference four-tuple demultiplexing, bounded listener/backlog
  admission, active ephemeral ports, stream readiness, fair polling and
  generation-checked retirement. Exercise two concurrent incoming flows and
  backlog refusal on the E1000 fixture.
- [x] Reuse the connection table and route selection in stock netd for
  explicitly granted outbound native TCP sockets. Verify the active path over
  E1000 and leave application listener admission open.
- [ ] Test an initial congestion algorithm such as Reno before Internet release.
- [ ] Define backlog/accept, ephemeral ports, SO_REUSEADDR/keepalive/TCP_NODELAY,
  quotas and rare-option boundaries.

Acceptance: multi-MiB streams under loss/reorder/duplicates, handshake/
zero-window/reset/simultaneous-close/TIME_WAIT/SYN exhaustion without leaks.

### N4 and N5 — native sockets and musl adaptation

The [approved application socket boundary](docs/network/application-sockets-proposal.md)
defines the initial outbound IPv4 scope. See
[native socket behavior](docs/network/application-sockets.md) for the verified slice.

- [x] Expose nonblocking IPv4 TCP/UDP descriptors to explicitly granted boot
  clients. Share native I/O, poll and wait-set across sockets, with bounded
  requests and receive credit.
- [x] Exercise native TCP partial I/O/EOF, UDP truncation/copyout retry,
  process isolation and old socket wakeup when netd retires its bridge.

- [ ] Define versioned native open/bind/connect/listen/accept/send/receive/send-to/
  receive-from/shutdown/options/resolver APIs with typed IPv4/IPv6 endpoints.
- [ ] Preserve partial I/O, EOF/reset/timeout/would-block/cancellation and common
  socket/IPC/timer/file readiness.
- [ ] Separate CONNECT/BIND_LOW_PORT/LISTEN/RAW/CONFIGURE_INTERFACE/OBSERVE_ALL rights.
- [ ] Bound per-process sockets/queued bytes/DNS/backlog, and define deterministic
  transfer/revocation and netd-restart errors.
- [x] Provide an opt-in static musl C adapter for nonblocking outbound IPv4
  TCP/UDP, endpoint and error queries, datagram truncation and mixed poll.
- [ ] Integrate socket translation into the libc fork and add blocking,
  listener, IPv6 and resolver operations as their native contracts mature.

Translate POSIX socket calls, sockaddr/options/flags/errors and getaddrinfo/
getnameinfo through native adapters, never Linux structs. Blocking/nonblocking
poll/pselect waits share native readiness; unsupported options return ENOPROTOOPT.
Acceptance: native Rust and static C echo/DNS/HTTP clients share conformance for
partial sends, close-during-wait, nonblocking connect, quotas, transfer, resolver
concurrency and service restart. N4 design can accompany N1–N3; freeze after
working semantics. N5 may use host reference mocks before native integration.

### N6 — TLS, HTTP and browser network service

- [ ] Supply secure entropy and an updateable read-only certificate store with
  version/rotation/rollback.
- [x] Provide valid-boot-UTC realtime; missing boot UTC returns an error.
- [ ] Port established TLS/crypto rather than inventing cryptography.
- [ ] Implement bounded HTTP/1.1 redirects/chunking/content-length/compression/reuse;
  HTTP/2 follows validated TLS/TCP.
- [ ] Isolate browser network origin/proxy/download policy, quotas/trust and bounded
  atomic cache/cookies in a sandboxed service with IPC/shared-buffer response bodies.
- [ ] Deny ambient sockets to renderers/content; browser UI uses ordinary GUI rights.

Acceptance: deterministic local HTTPS/test-root fetch then GUI response viewer;
expired/wrong-host/invalid certificates, redirect loops, truncated framing,
decompression bombs, corrupt cache and network-service crash/restart. Full
HTML/CSS/JS engine is M10, not a prerequisite for netd.

### N7 — IPv6 and multiple interfaces

- [ ] After IPv4, implement multicast/IPv6/ICMPv6/NDP/DAD/link-local/SLAAC/RA/DNS;
  add DHCPv6 only for demonstrated need.
- [ ] Reuse socket/UDP/TCP with per-interface addresses/routes/DNS/metrics/failover.
- [ ] Add Happy Eyeballs in resolver/client policy after dual-stack.

Acceptance: IPv4-only/IPv6-only/dual-stack use the same app APIs.

### Configuration, security and network acceptance

Proposed paths: `/system/share/network/defaults.toml`,
`/system/services/netd.toml`, `/users/<uid>/config/network`, bounded
`/var/{lib,log}/netd`, volatile `/run/netd`. Versioned configuration validates,
stages and applies atomically with rollback. Status reports links/addresses/
routes/DNS/counters without exposing other traffic; tools provide status/route/
ping/DNS/connect and privileged bounded capture. Deny raw/interface/low-port
privileges by default; manifests request scopes. Bound packets/fragments/socket
buffers/DNS/TCP states/logs and fuzz every parser. Firewall/VPN needs later
routing hooks/key storage and separate acceptance.

Order N0→N1→N2→N3→N4→N5→N6→N7. Matrix through the harness uses an isolated
network, deterministic local servers, loss/reorder/duplication/corruption/latency,
malformed corpus/fuzz, netd/NIC restart with active sockets, Rust/C parity,
HTTPS/test-root/GUI response and connect-close/DNS/retry leak soak. Optional
public-Internet release smoke does not replace CI. Networking v1 closes only
when E1000 real I/O, DHCP/DNS/TCP, both app APIs, sandboxed HTTPS and restart/
malformed-traffic containment all work; only permitted config/state survives reboot.

### Other platform services and packages

- [ ] Provide HDA discovery/DMA, userspace mixing and per-app streams/permissions.
- [ ] Complete xHCI enumeration/descriptors/endpoints, HID keyboard/mouse, USB mass
  storage and hotplug.
- [ ] Add modern VirtIO block/net and interrupt-driven transport with I/O/reset/feature tests.
- [x] Implement reboot/poweroff.
- [ ] Add ACPI battery/thermal/suspend/resume.
- [ ] Publish one reference PC and tested QEMU devices before broader driver tiers.
- [ ] Treat Wi-Fi as future hardware, not a first desktop-release dependency.

PCI discovery alone is not working device support.

- [ ] Define native signed bundles shared by Rust/C and Minimal/DWM, built
  reproducibly from SDK/app manifests.
- [ ] Specify ID/version/architecture/minimum ABI, executables/resources/services/
  GUI version, bounded dependencies and requested capabilities.
- [ ] Record data/config/cache ownership, hashes/signatures and publisher/trust.
- [ ] Support offline local packages first, validate before writes and perform
  stage→verify→activate→rollback with a persistent local database.
- [ ] Make install/update/remove interruption-safe and preserve user data on uninstall.
- [ ] Add online repositories after storage/network/TLS/time/trust/key rotation.
- [ ] Reject incompatible ABI or missing authority before activation.
- [ ] Keep maintainer scripts free of ambient privileges; do not use Linux package
  formats as the platform compatibility strategy.

All platform services need real I/O, denial and recovery acceptance.

## M9 — useful SDK applications

- [ ] Build rock: a nano-level editor with open/edit/save, cursor/selection, search/
  replace, undo/redo and status/shortcuts; TUI first, then GUI on the same buffer.
- [ ] Provide a GUI text editor.
- [ ] Provide an SDK image viewer with Files integration, zoom/fit, shared BMP then
  PNG/JPEG decoders; replace raw bmp_viewer only after parity.
- [ ] Provide a read-only text/Markdown viewer with shared typography.
- [x] Implement existing GUI configuration/themes/Lua/SVG/SDK transfers.
- [ ] Continue application usability and remaining migration as separate work.

Depends on M7 widgets/themes/tools/C SDK and M6 on-device TCC/file/spawn.
Acceptance: cargo dunit build/run, standard SDK controls without custom duplicate
paint/hit-test, keyboard navigation/mnemonics, function markers and both-theme
visual goldens. Rock edit→save→reread matches a hash; image viewer opens known
dimensions. Replace raw bmp_viewer only after parity. Close the rock→save C source
→TCC compile/link→native launch cycle entirely within QEMU, including an actual
C GUI window. At least rock, editor and image viewer must be usable; do not expand
the app list indefinitely.

## M10 — browser

- [ ] Choose browser scope/engine with the human: a minimal HTTPS text/simplified-
  HTML viewer without JS, an existing lightweight engine port, or a new engine.
- [ ] Define acceptance for that choice before committing to HTML/CSS/JS scope.
  Do not preselect NetSurf; porting/writing an engine has substantial dependency,
  security and artifact-size costs.

Requires M7, M8 netd/TCP/DNS/TLS/sandboxed network service, secure entropy/trust/
valid clock, text shaping/kerning/wrapping/complex scripts and PNG/JPEG/GIF/WebP
codecs. HTML parsing, CSS cascade/layout and JS are separate substantial work.
Acceptance of the first slice: local HTTPS/test-root page loaded through sandboxed
service and displayed through normal GUI surfaces, with parser fuzzing and crash/
privilege containment. Engine scope cannot block network/SDK delivery.

## Images, release and remaining engineering work

Minimal and DWM share kernel/ABI/package formats and differ by manifests.
Minimal contains init, recovery console/storage/fs tools and optional network
diagnostics; DWM adds compositor/runtime/desktop/apps/fonts/themes. Live adds
installer/recovery payload; installed profiles ultimately use persistent root.
Current editions and build composition are documented in [image builds](docs/build-images.md).
Sysroots/images rebuild from pinned compiler/fork/ABI sources, not opaque blobs.
Reproducible releases, provenance/license metadata, ABI cadence, compatibility
and upgrade rollback remain release goals.

- [x] Share Minimal/DWM kernel, ABI and image-composition manifests.
- [x] Remove raw VFS backend/global-path/task-reference escapes in OP1–OP3 and
  centralize protocol definitions.
- [x] Remove fake kernel package/thread/device/proc backends, duplicate asset reads
  and the unused bootloader crate; consolidate English manuals and review findings.
- [x] Remove unbuilt Multiboot entry/linker files and the broken GRUB image target.
- [x] Retire initial duplicate kernel test models and bootimage runner; test the
  real heap and ELF parser, including growth rollback and malformed headers.
- [x] Preserve exact asset/overlay bytes when packaging ELF data; strip only
  declared applications and system tools.
- [x] Fix bounded asset reads, Unicode strings, malformed padding/weighted grow and
  invalid subtree translation; verify host and native behavior.
- [x] Check the kernel and all full-profile Rust apps against a zero-warning budget;
  use Makefile's compiler/app list and reject budget increases through `--update`.
- [ ] Split oversized kernel/syscall/process/compositor modules by responsibility.
- [ ] Finish safe ownership/locking and a recovery-console-only Terminal path.
- [ ] Extend warning coverage to unpackaged Rust crates and C code.
- [ ] Version remaining configuration/filesystem/release contracts.
- [ ] Add structured log levels and bounded persistent logs.
- [ ] Provide GDB stub, tracing/profiling/crash dumps.
- [ ] Add accessibility/shaping/IME/localization.
- [ ] Add swap after robust VM/storage, and GPU after measured software limits.

Must-have release foundations are truthful contracts, runtime isolation/events,
GUI composition/parity, recoverable disk-root/init and persistence. SDK/native
static portability are the next application layer; networking/apps/browser,
advanced hardware, signals/fork/dynamic tier and GPU are separately gated.
Avoid simultaneous scope expansion or temporary ABI freeze. Protect against
lock/IRQ races, shared-buffer leaks, filesystem corruption, premature protocol
freeze, hidden Linux assumptions and wrong-disk installation with vertical slices,
capability bounds, conformance/fault tests and explicit device authorization.

Dependency path: M0→M1 and M2→M3→M4; M5 supplies durable M4/M6 userspace;
M1→M6; M4/M6→M7; M7→M8 and M9; M7/M8 plus text/codecs→M10.
Protocol/layout/source audits and storage design can proceed independently,
but integration waits for approved object/ABI contracts. OP0–OP9 is the independent
SMP/performance track; AP dispatch waits for its safety gates. A reliable first
UP software-composited installed desktop need not wait for SMP/GPU/dynamic/Wi-Fi
or a full browser.
