# Current system state

Source review: 15 November 2026, before OP4. This page describes implemented
behavior and its limits. Test contracts describe how to verify it; their presence
is not evidence of a fresh run. The [technical roadmap](../../DUNIT_OS_TECHNICAL_ROADMAP.md)
and [performance roadmap](../../DUNIT_OS_PERFORMANCE_ROADMAP.md) own future work.

## Architecture

Limine boots the Rust Green Tea Kernel through a C/NASM HAL on x86_64.
HAL owns CPU entry, descriptor tables and context switches. The kernel owns
memory, tasks, scheduling, IPC, handles, VFS and device mechanisms. Applications
use the generated [native Dunit ABI](../../abi/README.md); musl translates POSIX
source interfaces into that ABI.

Userspace `init` supervises manifest-defined services. `gui_server` owns the
display/input master handles and hosts the compositor and DWM shell. Keeping the
shell there is the accepted architecture; applications retain separate address
spaces. Desktop lists, themes and rules come from userspace configuration.
The kernel recovery terminal and early boot display remain diagnostic bootstrap
paths. They must continue to work when userspace initialization fails.

## Implemented behavior

| Area | Behavior | Verification contract |
|---|---|---|
| Boot | BIOS/UEFI, Limine framebuffer and external initrd | `tools/boot_markers.json`, `tools/test_image_profiles.py` |
| Memory | PMM-backed growing heap, isolated address spaces, guarded VM, W^X, shared buffers and eager private file mappings | `tools/boot_markers.json`, `tools/test_user_memory.py` |
| Tasks | Timer preemption, per-thread GPR/kernel stack/FXSAVE/static TLS, spawn/wait/kill, fault isolation, futex waits | `tools/boot_markers.json`, `tools/m6_thread_lifecycle_markers.json` |
| Ownership | Task registry pins, exclusive execution admission, deferred dispatch/fd retirement, locked PTY queues and scoped console/stdin transactions | `tools/test_process_resources.py`, `tools/test_pty.py`, `tools/boot_markers.json` |
| Waiting | Blocking timer/IPC/futex and level-triggered mixed IPC/fd/timer/child wait-set | `tools/waitset_markers.json`, `tools/test_wait_queue.py` |
| Files | MemFS root, initrd apps/assets, task snapshots, DunitFS v2 at `/persist`, fd duplication/pipes/flags/spawn actions | `tools/m6_fileops_markers.json`, `tools/m6_native_fd_markers.json` |
| Devices | PS/2 input, queued pointer input, PCI, framebuffer/Bochs display modes, AHCI and virtio-blk | driver-specific and GUI marker contracts |
| Runtime | Static native Rust SDK, static Dunit musl, on-device TinyCC and full Lua interpreter | [SDK](../sdk/README.md), [libc tests](../libc-test.md), [Lua](lua-runtime.md) |
| Desktop | Configurable panels/components, themes, input/focus, text clipboard, scoped file transfers, window motion and isolated Lua workers | [desktop manual](desktop.md), `tools/gui_smoke_markers.json` |

Application dispatch is UP: APs run bounded initialization/ownership/TLB probes
and then park; `dispatch_cpus=1`. This does not demonstrate concurrent application
execution. FXSAVE covers x87/MMX/SSE; AVX/XSAVE is outside the supported profile.
On q35, HPET supplies monotonic time and PIT supplies 100 Hz scheduling events.
Realtime uses Limine's UTC boot timestamp plus monotonic elapsed time; an absent
boot timestamp returns an error. Desktop clock widgets still display uptime.

Root `/` is MemFS populated from initrd. DunitFS `/persist` stores persistent data;
an installer and successful data readback do not establish a disk-backed system
root or booting init from disk. Device discovery and proc diagnostics do not
establish a mounted general-purpose DevFS/ProcFS interface.

## Pre-OP4 review

The review followed declarations, callers and history across kernel/HAL, VFS,
task/PTY ownership, runtime/userspace and first-party documentation. It removed
unused kernel package-catalog and thread-runner simulations, an unmounted DevFS
that discarded writes while reporting success, an unmounted ProcFS with no EOF
progress and invented stat sizes, an empty keyboard initializer,
unused shell fields and an obsolete bootloader system sketch. `dtop` now uses
SystemStats memory counters and reports CPU usage unavailable; `--once` provides
a terminating snapshot. Asset reads now reject incomplete/oversized candidates, quoted DUI strings
preserve UTF-8, padding rejects malformed fragments, weighted grow sizing works,
and subtree translation rejects invalid IDs/layout sizes.
No syscall, right, wire record or disk format changed.

Markdown is consolidated into current manuals and two roadmaps. Historical
proposal/status documents no longer form competing sources of current truth.
Upstream license/provenance text and JSON measurement receipts are retained.
Compatibility entry points are retained where callers still require them.

## Open gates

- [OP4 ownership gates](../performance/smp.md#pre-op4-gates): console/stdin scopes
  and the former global VMM getter are addressed. Display-master/modeset lifetime,
  driver DMA retirement, remote cancellation, lock ranking/IRQ progress and real
  concurrent VM/application stress remain open before AP dispatch.
- Secure entropy is absent. ELF AT_RANDOM bootstrap bytes are not a CSPRNG and
  must not support cryptography or a claim of production hash hardening.
- Signals/interruption, cancellation, process-image exec/fork, dynamic loader/DTV,
  full termios, POSIX ownership/accounts and disk-backed system root remain open.
- Networking needs packet RX/TX, native socket objects/rights/readiness and DNS.
  HTTPS additionally requires entropy and TLS. Network-controller discovery is
  not a working network stack. Audio, full USB enumeration and power services
  remain incomplete; xHCI compatibility heuristics need real-device validation.
- Eager file MAP_PRIVATE is a snapshot with EOF/W^X limits; no demand paging,
  COW, MAP_SHARED or SIGBUS semantics are claimed. Directory iteration is a
  bounded snapshot, not a persistent fd-backed directory object.
- GUI send success does not prove completion of every RESULT/CONFIGURE/FRAME_DONE
  transaction. Full resize acknowledgment/rollback remains a protocol-state gate.
- Previous-release binary fixtures, upstream upgrade rehearsal and reference-PC
  validation remain distinct release gates. QEMU results prove their named scope.

## Verification

All OS builds and boots use `tools/qemu_test.py`; shared builds are serialized.
Use `tools/test_process_resources.py` for teardown/fd/thread regressions, the boot
manifest on 1 and 8 CPUs, and GUI plus production manifests on 4 CPUs. Host ABI,
protocol, PTY, wait/IPC, task-access and build-tool checks complement native runs.
Read the full serial log for readiness, exits, faults and resource cleanup.
Warnings are tracked by `tools/check_warnings.py`; budgets must not increase.

The pre-OP4 branch passed native resource/thread suites on 4 CPUs, full boot on
1/8 CPUs, GUI smoke on 4 CPUs, production core/IPC/time on 4 CPUs, and the SDK
file/entry/time/child suite plus dtop snapshot. Required markers were present and
forbidden markers absent. Host ABI/protocol, PTY, task access, wait/IPC,
user-memory, harness/tooling, image/configuration and DWM/component regressions
also passed. Bounded-reader tests cover short reads, exact limits, oversize,
mid-read EIO and invalid transfer counts; native cases verify fd cleanup.

Kernel production/smoke checks emitted zero warnings. The warning checker's
listed userspace subset also measured zero, and its budget was lowered to zero.
That subset does not cover every packaged app; existing warnings in other
apps are not evidence of a clean whole-repository warning inventory. Fresh
receipts are under `build/performance/pre-op4-*` and
`build/performance/pre-op4-review/`; they are generated artifacts, not source docs.

## Historical validation

These are prior results, not new proofs for an arbitrary later change:

| Date / source | Scope |
|---|---|
| 24 October, M6 integration | One regression run: 40 contracts, 73 commands, 175 required markers, zero forbidden. Static C ABI, fd/PTY/lifecycle, musl threads, TCC/Lua, themes/SVG and desktop policy; other suites were separate. |
| 24 October, thread stress | 1000 joinable and 1000 detached threads with TLS/errno/malloc/yield; both completed without previous OOM. |
| 24 October, persistence | 131072-byte musl write, shutdown and raw-image reboot/readback at `/persist`; hash `b4c071280fb62325`. |
| 24 October, reproducibility | Offline clean-fork libc archive reproduced SHA-256 `3941cd27e27ff606698b8329a2d290c7db06afc8c9bd0b8ee78cf528be1874fa`. |
| 24 October, GUI | Separate 512/768/1024 MiB smoke matrix, production 3K mode switching and SVG UI; Lua failure/timeout fallback checked separately. |
| OP3 published base `f713fc7` | PTY generation/endpoint ownership, task/fd retirement and native resource/boot/GUI/production suites; [retained receipts](../performance/smp.md#verification). |

Original M6 source findings used revision `36e85ce`. Subsequent fixes covered
literal-path kernel spawning, userspace command resolution, versioned spawn
records, strict unsupported libc errors, realtime validity, static TLS auxv,
thread reclamation, MemFS unlink identity, shared-buffer release and configuration
validation. Old line numbers and unaccepted recommendations at that revision
must not be read as current failures or pending M7 decisions.
