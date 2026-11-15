# Dunit OS technical roadmap

Updated 15 November 2026. This is the canonical architecture, dependency and
acceptance plan. Implemented foundations do not close an entire milestone:
M4 persistence, M5 installed root, M6 release gates and M7 SDK completion remain
open. [Current state](docs/architecture/current-state.md) describes the implementation;
[performance and SMP](DUNIT_OS_PERFORMANCE_ROADMAP.md) tracks OP0–OP9 separately.
Historical reports are evidence for their recorded commits, not fresh verification
of today's tree. Git history retains the superseded roadmap and vault.

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
contracts first. Kernel NIC drivers will feed bounded packet queues to userspace
`netd`; browsers will use a separate sandboxed network service.

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
| Memory | PMM/VMM/growable heap, isolated ELF, safe user copy, anonymous private mmap/unmap/protect, W^X/guards/shared objects | File mappings, demand paging/COW, dynamic loader/TLS, swap |
| Runtime | Preemptive UP, schedulable threads, FXSAVE x87/MMX/SSE, static TLS, futex, wait queues, clocks | Concurrent AP applications, priorities/accounting, AVX/XSAVE, signals/interruption, process-image exec/fork |
| Objects/ABI | Generated native ABI, bounded IPC with sender identity, narrowing-only handles, display/input exclusivity | Public release compatibility suite, edge/persistent wait-set, future network contracts |
| Files | VFS/MemFS root, fd/cwd/file APIs, dup/pipe/fcntl/poll/ioctl and atomic spawn actions | Independent complete DevFS/ProcFS contracts, disk root, ownership/groups/permissions and links |
| Storage | AHCI DMA, legacy VirtIO polling, GPT policy/CRC, transactional installer, DunitFS v2/fsck/degraded mount | 64-node limit, 1,000-file acceptance, broader recovery/hardware and user provisioning |
| Desktop | Userspace compositor/DWM, TOML/DSS/Lua, shared themes, SDK GUI apps, clipboard/DnD, pointer modes | Session geometry across reboot, declarative whole-window effects, frame budget, IME/a11y/shaping |
| C | Static Dunit musl, native pthread subset, TinyCC on-device compile/link/run, thin generated C UI v1 | Secure entropy, broad POSIX/release gates, dynamic tier, in-memory TCC `-run` policy |
| Hardware | PS/2, PCI, framebuffer, AHCI/VirtIO, reboot/poweroff, AP boot/park and HPET/IPIs | NIC packet I/O, complete USB/audio, modern VirtIO, ACPI battery/thermal/suspend |

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

Implemented: factual snapshot, ABI documentation, honest boot logs, CI warning
budget and required/forbidden markers. Maintain a source/test link for every
working claim and versioned decisions for process entry, handles, GUI, libc and
future services. Acceptance: repeatable Terminal/GUI/BIOS/UEFI baseline with no
claim based only on source-file existence; compiler warning budget may decrease,
not silently grow. A historical snapshot must state its date and limits.

## M1 — kernel runtime prerequisites

Implemented: default UP preemption proven with CPU-bound parent/child without
yield; per-thread GPR/kernel stack/FXSAVE and shared process address space/fds;
thread create/exit/join and fault isolation; blocked sleep/IPC/timeouts; anonymous
VM partial unmap/protect, guards/W^X and shared-frame teardown; Variant II static
PT_TLS/FS.base; atomic compare-and-park native futex with timeout; capability
memory/endpoints/display/input and narrowing-only transfer. Mutable globals have
moved toward owned/locked/atomic state; their SMP safety is tracked in OP1–OP3.

Depends on M0. Acceptance: preemption/starvation stress, distinct XMM/TLS/errno,
wait/wake races without lost wakes or busy waiting, mapping boundaries and kill/
exit resource cleanup. IRQ/lock inversion, FPU leakage and use-after-unmap must
be tested. Dynamic TLS/DTV, AVX, accounting/priorities and SMP are separate gates.

## M2 — GUI protocol and headless reference models

Implemented: versioned `gui-v1` binary specification/codec and deterministic
headless models for framing/negotiation/errors, serials/namespaces, surface/
buffer/configure/ack/commit lifetimes, frame callbacks, focus/input/grabs,
geometry/quotas and seeded malformed-byte/event tests. Useful pure window/
composition models were retained; the old egui process lifecycle was removed.

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

Implemented: exclusive display/input/shared-buffer mechanisms, ordinary ELF
`gui_server`, two isolated clients, software composition, frame lifecycle,
focus/input routing and removal of legacy kernel GUI/GUI1/dead display-server
crates. Live routing uses kernel-authenticated sender identity and the common
protocol codec; no magic PID or per-client wire copies.

Depends on M1/M2. Acceptance: overlapping clients, resize storms, reuse, input
isolation, crash cleanup and 10-minute idle/load. Malformed clients disconnect,
stale objects fail explicitly and one app crash removes only its surfaces.
GUI-service failure must leave the kernel/recovery console usable; supervision/
restart/fallback is part of service integration. Do not rewrite composition in
the application widget layer.

## M4 — configurable DWM and UI runtime parity

Implemented: independent `runtime/` crates for TTF text, DUI retained tree/two-pass
layout, DSS cascade/tokens/states/motion, widgets and software paint; interactive
windows, panel/dock/launcher/tray, quick settings, notifications, workspaces and
switcher; terminal/Files/Calculator/Monitor/Settings clients and userspace `dsh`
over PTY. `setup` and Settings share validated configuration. Lua/luac run on
Dunit; sandboxed policy/event scripts use deadlines and preserve working geometry
on error. TOML is base policy, DSS appearance, Lua behavior/native motion.

Configuration and assets are external: app/widget manifests, fonts, wallpaper,
five Chameleon icon themes and shared static SVG/bitmap lookup with scale/aliases/
fallback. Pointer geometry and mode switching through 3072×1920 are checked.
Available static SVG features do not imply full browser SVG support.

Use stable UI IDs and Row/Column/Stack/Grid/Scroll constraints, common widgets,
capture/bubble events, keyboard focus, cascade defaults→theme→class→state and
native property interpolation. TOML/DUI/DSS schemas need independent versions,
size/depth limits and offline linting. Reload is parse→validate→resolve→dry
layout→atomic swap; retain last-known-good on failure. Configuration must change
runtime behavior, not just parse successfully. Filesystem watching remains a
future event facility; explicit reload exists.

Open: session/window geometry persists across reload/reconnect only, not proven
across reboot; complete persistent user profiles depend on M5. Accessibility,
complex text shaping/IME and whole-scene frame budgets are incremental gates.
Acceptance: full feature parity, keyboard-only use, malformed reload rollback,
1024×768/1600×900/HiDPI visual/layout tests and animation timing; persistent
configuration/session hashes across stop/start. Appearance changes require no
kernel rebuild. Brightness UI eventually uses display/power service control.

## M5 — installed root, provisioning and recoverable storage

Partial. Implemented: GPT ESP/Dunit System/Data policy excludes foreign partition
types; real initrd and service manifests; supervision/restart budgets and
`/persist/.provisioned`; DunitFS v2 bitmap/extents, directories, rename/unlink,
timestamps/mode, fsync, generations/metadata COW, fsck and degraded read-only
mount; transactional validate→partition→format→copy→verify→boot-config→sync
installer with rollback; AHCI/VirtIO and BIOS/UEFI paths; distinct image/profile
manifests. Partition/device identity, bounds and explicit destructive authorization
remain mandatory for real installation.

Open: disk-backed root with init/services/apps/assets read from system volume;
read-mostly system enforcement, complete system/user provisioning; ownership/
groups/permissions, links/orphan cleanup and scalable metadata beyond 64 nodes;
1,000-file and broader interruption/recovery acceptance. Initrd extraction into
MemFS and a writable `/persist` do not close the milestone.

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

Toolchain: logical `x86_64-dunit`, static compiler-driver/LLD flow, crt1/crti/crtn,
public headers/libc.a, native headers, compiler/ar/strip wrappers and SDK version/
license manifest. Reject host headers/libraries. CMake/Meson/Autoconf cross files
and fingerprint-driven sysroot rebuilds exist. Clean/offline rebuild, artifact
hash/upgrade rehearsal and published versioned capability profile remain release
requirements. Full GCC or Rust-on-device ports are not prerequisites.

Profiles and remaining gates:

| Profile | Implemented | Open acceptance |
| --- | --- | --- |
| A: startup/ISO C | crt/init/main/exit, 0/1/2, stdio, strings/math/conversion, constructors/args/env/auxv | Maintain startup/stack/bss/error/invalid-pointer conformance |
| B: VM/allocator/time | anonymous mmap/unmap/protect, allocator without Linux brk, zero-fill/alignment, monotonic and valid-boot-UTC realtime | Secure entropy; fragmentation/OOM/protection and clock edge cases |
| C: files | buffered FILE, open/read/write/seek/stat/cwd/directories/rename/unlink/fsync/truncate, flags/error adapters | Installed-root libc/tools, full ownership/account database and storage matrix |
| D: processes | native posix_spawn argv/env/cwd, pipes/dup/redirection/fcntl/poll/ioctl, atomic ordered open/close/dup2/chdir actions | Process-image exec/fork; unsupported spawn attributes remain errors |
| E: threads | native thread/TLS/futex, create/join/detach, errno, mutex/condvar/rwlock/once and reclamation | Broader primitives such as barrier, owner-death/cancellation/interruption policy |
| F: later | Official static-only build policy | Signals/timers/interruption, sockets after netd, dynamic loader/TLS and demand-driven fork/COW |

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
Previous-release binaries and frozen ABI/libc release contracts remain open.
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

### M7.1 — wait-set

Implemented `WaitSetWait` (89): call-scoped level-triggered IPC, timer/deadline,
file/pipe/PTY and child-exit readiness, safe wrapper and owning event loop;
tokens, timeout, fd identity/deadline preservation and cancellation via IPC.
Previous IPC/futex/wait/poll calls remain compatible. Open: edge/persistent
registration and broader integration; socket readiness follows M8.
Acceptance: pipe/IPC/timer wake returns the correct token, close produces hangup,
timeouts/lost-wake races pass and real event loops block without polling.
Depends on M1; future contract expansion needs approval.

### M7.2 and M7.3 — runtime, tooling and app manifest

Implemented CLI/TUI/GUI `cargo dunit new/build/run/test` wrapping the existing
harness, native `#[dunit::main]` argv/env/panic/exit and compiler-collected tests
(empty/Err/panic fail), owning fs/process/time/io/env/args/stats APIs and shared
Error/errno. `dunit.toml` v1 strictly declares identity/name/icon/kind, exact Cargo
binary target and unique requested capability names; declarations grant no
rights. Packaging and a shared userspace catalog feed launcher/dock/startup.
CLI/TUI use configured Terminal with separate argv/PTY, never shell interpolation.

Open: standalone SDK distribution, remaining legacy catalog migration, thread/
sync APIs and the future `dunit::net` backend. Acceptance: generated app has no
unsafe/extern-C/raw entry, cross-builds/packages/runs with checked output/window;
`cargo dunit test` supplies its own markers. At least one migrated real app uses
this route. Capability declaration, cwd/env, spaces in paths, CLI/TUI input,
child cancel/exit and cleanup must retain native checks.

### M7.4 and M7.5 — typed UI, shared styles and effects

Implemented owning Component/update/view, retained stable control keys,
layout/style/tree diff, old/new damage repair, clipped repaint, shared-buffer
reuse and no-op frame skipping; common Window/ComponentWindow manages configure/
resize/input/frame/close and teardown. Stable opaque/unscaled/unobscured scenes
use real texture/composition/present damage; overlap, effects and scene changes
fall back to full composition. Existing gui-v1 wire format is retained.

Shared typed DSS tokens supply colors, fonts, spacing, radii, borders, gradients,
shadows, clips and client-canvas backdrop blur. Light/dark reload preserves models
and last-good appearance. Stable-key timed appearance transitions use wait-set
sampling, retargeting and reduced motion. Open: nested component lifecycle,
geometry/full-widget animation coverage, declarative whole-window transparency/
blur/rounding without manual ARGB, adaptive asset/manifest packaging and expanded
visual references/static SVG features. Complex shaping/IME remain later work.

Acceptance: message sequences prove model/tree state; incremental/full pixels
match, old shadows/blur sample regions repaint correctly, tiny changes bound
composed pixels and no-op sends no frame. Default apps in both themes pass pinned
font/scale goldens with no private appearance literals; theme change repaints all
token areas. Whole-scene transparent-window ≤16 ms remains an unachieved target,
tracked with reproducible performance evidence rather than average FPS.

### M7.6 — widget behavior

Implemented families: push/toggle/check/radio/switch; controlled Input/Textarea
with grapheme/word/line selection, soft wrap, undo/redo, password/readonly/limits;
shared two-axis scroll/selection autoscroll/auto-hide; slider/vertical slider,
integer spinbox, progress/busy; select, menu bar/submenus/context/tooltip;
tabs/splitter/toolbar/status/dialog/modal/message box; virtual list/tree/table,
stable selection/expansion/sort/F2 rename and Open/Save/Directory file picker.
State-aware flex/grid, capture/bubble with stop/prevent, scoped mnemonics/roving
focus and lifecycle cancellation use common paint/hit geometry and styles.

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

Implemented common protocol/control codecs (`CAP1/INP1/RLD1/NOT1`, centralized
GUIC) and generated XFR1 transfer values. Live committed-buffer damage reaches
CPU textures and framebuffer presentation; CI rejects duplicated control tags.

`gui_server` brokers typed text/files/images/custom-MIME clipboard/DnD,
copy/move/link negotiation, pointer capture/feedback/target highlighting and
on-demand shared-buffer data. File targets receive genuine attenuated file/
directory capabilities with owned/RAII lifetime; text targets receive quoted
paths. Transfers retain authorization/rollback across IPC and do not imply
revocation after completion. Files' private clipboard was removed.

Acceptance recorded complete: all 49 ordered source/target pairs among seven
production GUI apps, with new catalog entries failing incomplete matrix coverage;
real file contents/shell output, multiple files/Unicode/quotes, workspace crossing,
new-process open, source/target crash, clipboard persistence after source exit,
Escape/unsupported without data delivery, readonly/password fields and DunitFS
sync/reboot. Unsupported file Link or cross-backend Move remains explicit and
must not be imitated by copy. No duplicate wire definitions or handshakes.

### M7.9 — compositor shell decision

Completed branch: retain the shell in `gui_server`, using the shared DSS registry,
palette/metrics/cursor/shadow/control geometry and atomic candidate validation.
Settings saves only intentional scalar overrides, preserving DSS inheritance,
independent tables/comments and no-op files. Appearance CI gates exclude format
arithmetic/resource limits appropriately.

A normal external SDK-client probe established isolation/publication/boot/recovery,
not a complete shell replacement. External layers/struts/policy/restart contracts
are absent, so extraction is deferred. The compositor is not rebuilt on widgets.
Recorded 21 ms frame markers did not meet the ≤16 ms goal; input series include
slower runs and do not establish universal speedup/non-regression.

### M7.10 — thin generated C SDK

Implemented/accepted: `<dunit.h>` and `libdunit_ui.a` wrap the same Rust runtime,
widgets/styles/renderer. Generated C/Rust records, constants, callbacks, layouts,
exports and signatures come from `runtime/dunit-c/api.json` and existing ABI/
protocol owners. C owns model/update data; SDK owns input/focus/rendering with
bounded copied/borrowed lifetimes and error/reentry/reopen rules.
Core/checkable/radio/disabled, layout/styles/text/numbers/progress/menus,
collections/workspace/file picker and clipboard/DnD/file-capability tokens are
available. Additive C ABI v1 snapshots and an old object surviving forced Rust
rebuild/guest link-only execution protect compatibility. Native TCC compilation
and exact Rust/C pixel comparisons prove one toolkit. Image/ESP budgets remain
measured requirements, not permission to grow artifacts indefinitely.

Open independently: in-memory `tcc -run` execution/JIT policy. Existing RW→RX
mprotect does not approve a new executable-memory privilege model. File-based
`tcc -o` plus native spawn already works. Future C ABI changes follow version/
compatibility gates; do not duplicate the Rust toolkit.

### M7.11 — migration completion

Implemented: `calc`, `dufetch` and Monitor native APIs, portable `wc_demo` through
native C headers, all seven GUI clients on retained SDK controls/shared special
canvas. Old entry/panic, GUI handshake/buffers, standard controls, font loaders,
raw PTY and protocol copies were removed with before/after regression checks.
`dufetch` reads runtime facts; a versioned identity/uname source remains open and
must not be replaced by invented literals.

Open: `dsh`, `dtop`, `fsck_dunit`, raw image-viewer replacement and remaining
CLI/TUI/service lower-layer migration; thread/sync and standalone distribution
also prevent SDK completion. Preserve each program's existing outputs/markers
before removal of its old implementation; keep dynamic counters variable while
checking their format. SDK migration must improve usability while preserving
behavior, not merely rename wrappers. Final M7 acceptance requires real apps
built/run through cargo dunit, no duplicate raw entry/GUI lifecycle/hit-test and
remaining source-of-truth migration completed.

M7 order: wait-set/runtime decisions → tooling/manifest → typed UI/themes/widgets
→ shared protocol → transfers → compositor styles/decision → C bindings → full
migration. M7 supports M8 wait-set/network/service APIs, M9 apps and M10 browser;
independent design can proceed, but releases cannot bypass these dependencies.

## M8 — networking and platform services

Networking is unimplemented beyond PCI discovery and E1000 status/MAC;
`net0` is not packet I/O. First reference device is QEMU E1000. Modern VirtIO-net
follows a proven packet path; RTL8139 is later compatibility.

Kernel: PCI/MMIO, DMA-safe memory, NIC reset/IRQ/moderation, bounded RX/TX rings
and capability transfer. `netd`: Ethernet/ARP/NDP/IP/ICMP/UDP/TCP/DHCP/DNS,
interface configuration and policy. Native/socket/resolver protocols and musl
adapters converge on that service. Propose NET_DEVICE/PACKET_QUEUE and RX/TX/
CONTROL/RAW/TRANSFER authority only with human approval. Own buffer transitions
`DRIVER_RX → NETD → FREE` and `NETD_TX → DRIVER → COMPLETE`; bounded shared rings/
events avoid per-byte syscalls. On netd crash stop/revoke/reset/restart safely.
Ordinary apps never obtain raw NIC/MMIO/DMA authority.

### N0 — E1000 packet I/O

Open: reset/register/MAC/link setup, common DMA API, RX/TX descriptors and head/
tail/barriers/recycling, MSI/MSI-X with legacy fallback, bounded interrupt drain/
moderation; polling only for diagnostics. Handle malformed/oversize frames, link
changes, stalls/exhaustion/reset and counters for packets/bytes/drop causes/
checksums/full queues/resets/transitions. Acceptance: bidirectional frame path,
ring wrap/starvation/completion/link-toggle/interrupt-storm/reset tests and
10-minute flood without DMA corruption/leaks; guards and ownership assertions.

### N1 — Ethernet, ARP, IPv4 and ICMP

Open: checked Ethernet II sizes/padding/types, reject bad/unknown frames before
deep parsing; bounded ARP INCOMPLETE/REACHABLE/STALE retry/expiry/poisoning policy;
IPv4 version/IHL/length/checksum/TTL/protocol/address validation, connected/default
routes and longest-prefix selection; explicit MTU/TX-fragment and bounded-or-
deferred RX-reassembly policy; rate-limited ICMP echo/unreachable/time-exceeded
and logical loopback. Acceptance: ping gateway/host on ARP miss/hit/expiry,
bad lengths/checksums/TTL, route/ARP timeout, MTU/fuzz cases without service panic.

### N2 — UDP, DHCPv4 and DNS

Open: port demultiplexing/ephemeral allocation/checksums/bounded queues;
datagram bind/connect/send-to/receive-from, nonblocking/timeouts/readiness;
DHCP INIT/SELECTING/REQUESTING/BOUND/RENEWING/REBINDING with lease/link reset;
validate offers/options and atomically apply address/prefix/gateway/DNS with
rollback. Revalidate cached leases after reboot. DNS A/AAAA/CNAME, bounded
compression pointers/TTL cache, query validation/retry/timeouts and TCP truncated
fallback. Define static→DHCP→admin/user precedence. Acceptance: DHCP loss/NAK/
renew/duplicates, DNS NXDOMAIN/malformed/cache/timeout and UDP overflow.

### N3 — TCP

Open: active/passive CLOSED→TIME_WAIT state machine, 4-tuple/sequence/ack/flag/
header/checksum/window checks, SYN/data retries, RTT/RTO and duplicate ACK;
bounded send/receive/out-of-order queues, half-close/FIN/RST, monotonic timers
and wrap/jump handling. Start with one tested congestion algorithm such as Reno;
Internet release requires congestion control. Backlog/accept, ephemeral ports,
SO_REUSEADDR/keepalive/TCP_NODELAY, quotas and rare-option boundaries must be
explicit. Acceptance: multi-MiB streams under loss/reorder/duplicates, handshake/
zero-window/reset/simultaneous-close/TIME_WAIT/SYN exhaustion without leaks.

### N4 and N5 — native sockets and musl adaptation

Open: versioned open/bind/connect/listen/accept/send/receive/send-to/receive-from/
shutdown/typed-option and resolver lookup service APIs with typed IPv4/IPv6
endpoints, partial I/O/EOF/reset/timeout/would-block/cancellation semantics and
common socket/IPC/timer/file readiness. Rights separate CONNECT/BIND_LOW_PORT/
LISTEN/RAW/CONFIGURE_INTERFACE/OBSERVE_ALL; bound sockets/queued bytes/DNS/backlog
per process. Transfer/revocation and netd restart errors must be deterministic.

Translate POSIX socket calls, sockaddr/options/flags/errors and getaddrinfo/
getnameinfo through native adapters, never Linux structs. Blocking/nonblocking
poll/pselect waits share native readiness; unsupported options return ENOPROTOOPT.
Acceptance: native Rust and static C echo/DNS/HTTP clients share conformance for
partial sends, close-during-wait, nonblocking connect, quotas, transfer, resolver
concurrency and service restart. N4 design can accompany N1–N3; freeze after
working semantics. N5 may use host reference mocks before native integration.

### N6 — TLS, HTTP and browser network service

Open prerequisites: secure entropy, valid realtime and updateable read-only
certificate store with version/rotation/rollback. Port proven TLS/crypto;
do not invent cryptography. HTTP/1.1 needs bounded redirects/chunked/content-
length/compression/reuse; HTTP/2 follows validated TLS/TCP. A sandboxed browser
network process owns origin/proxy/download policy, quotas, trust, bounded atomic
user cache/cookies; it sends response bodies/status via IPC/shared buffers.
Renderer/content has no ambient sockets and browser UI remains an ordinary GUI
client without display/input/network master rights.

Acceptance: deterministic local HTTPS/test-root fetch then GUI response viewer;
expired/wrong-host/invalid certificates, redirect loops, truncated framing,
decompression bombs, corrupt cache and network-service crash/restart. Full
HTML/CSS/JS engine is M10, not a prerequisite for netd.

### N7 — IPv6 and multiple interfaces

Open after the stable IPv4 vertical slice: multicast, IPv6/ICMPv6/NDP/DAD,
link-local/SLAAC/router advertisements/DNS; DHCPv6 only for demonstrated need.
Reuse socket/UDP/TCP paths, with per-interface addresses/routes/DNS/metrics/link
failover. Happy Eyeballs belongs in resolver/client policy after dual-stack.
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

Open audio: HDA discovery/DMA→userspace mixer→per-app streams/permissions.
Open USB: xHCI enumeration/descriptors/endpoints→HID keyboard/mouse→mass storage/
hotplug. Modern VirtIO block/net and interrupt-driven transport need separate
I/O/reset/feature tests. Reboot/poweroff work; ACPI battery/thermal/suspend/resume
remain open. Reference QEMU devices and one published reference PC precede broad
hardware/driver tiers; PCI detection alone is not support. Wi-Fi is a future
hardware target, not a first desktop-release dependency.

Packages are open native signed bundles, shared by Rust/C and Minimal/DWM,
built reproducibly with SDK/app manifests. Specify ID/version/architecture/minimum
ABI, executables/resources/services/GUI version, bounded dependencies, capability
requests, data/config/cache ownership, hashes/signatures/publisher/trust. Offline
local packages first; validate before writes, stage→verify→activate→rollback,
persistent local database and interruption-safe install/update/remove. Uninstall
must never silently delete user data. Online repositories depend on storage,
network/TLS/time/trust/key rotation. Reject incompatible ABI/missing authority
before activation; no ambient-privilege maintainer scripts or Linux package-format
compatibility strategy. All platform services need I/O and denial/recovery tests.

## M9 — useful SDK applications

Open new apps: `rock` nano-level editor (open/edit/save, cursor/selection, search/
replace, undo/redo, status/shortcuts), TUI first then GUI over the same buffer;
GUI text editor, SDK image viewer (Files open, zoom/fit, shared BMP then PNG/JPEG
decoders), read-only text/Markdown viewer with shared typography. Existing GUI
polish/config/themes/Lua/SVG/SDK transfers is implemented; further application
usability and full remaining migration are separate tasks.

Depends on M7 widgets/themes/tools/C SDK and M6 on-device TCC/file/spawn.
Acceptance: cargo dunit build/run, standard SDK controls without custom duplicate
paint/hit-test, keyboard navigation/mnemonics, function markers and both-theme
visual goldens. Rock edit→save→reread matches a hash; image viewer opens known
dimensions. Replace raw bmp_viewer only after parity. Close the rock→save C source
→TCC compile/link→native launch cycle entirely within QEMU, including an actual
C GUI window. At least rock, editor and image viewer must be usable; do not expand
the app list indefinitely.

## M10 — browser

Open. The first task at this milestone is a human choice of scope/engine:
minimal HTTPS text/simplified-HTML viewer without JS, lightweight existing-engine
port, or a new engine. Do not preselect NetSurf or promise a complete engine.
Define acceptance with that choice; porting and writing HTML/CSS/JS each have
large dependency/security/size costs.

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

Remaining cross-cutting work: split oversized kernel/syscall/process/compositor
files by responsibility; reduce existing warnings within CI budgets; finish safe
ownership/locking and recovery-console-only Terminal path; version all remaining
config/FS/release contracts; structured levels and bounded persistent logs; GDB
stub, tracing/profiling/crash dumps; accessibility/shaping/IME/localization;
swap after robust VM/storage; GPU after measured software correctness/limits.
OP1–OP3 already replaced raw VFS backend pointers/global paths and task references;
completed protocol centralization must not remain listed as missing debt.

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

## Human decisions

Accepted: native no_std+alloc SDK; call-scoped WaitSetWait; queued pointer input
and negotiated lifecycle/modifier/mode additions; manifest v1 with requested
capabilities; path-to-text and attenuated file/directory transfer authority;
compositor-hosted shell/shared DSS; thin generated C runtime with additive v1
compatibility. Preserve existing approved contracts.

Pending: edge/persistent wait-set and identity/uname API, future network object/
rights/scopes, new executable-memory/TCC-run policy, signal/fork/dynamic contracts,
on-disk root/format/privilege changes and browser engine/scope. Present options
and recommendations when required, proceed on independent work and do not treat
this roadmap as blanket approval. The browser decision belongs at M10; existing
RW→RX alone is not approval for a new JIT policy.
