# Dunit OS contributor contract

This contract applies to every agent and every part of the repository, including
delegated work. The human sets architecture and system policy; agents implement,
verify and repair authorized work independently. User instructions take
precedence, including permission already granted in the session.

## Repositories and architecture

[DunitOS/dunit-os](https://github.com/DunitOS/dunit-os) is the main OS repository
(default branch `main`). [dunit-musl](https://github.com/DunitOS/dunit-musl) and
[dunit-tcc](https://github.com/DunitOS/dunit-tcc) are complete upstream forks on
`master`, pinned at `toolchains/dunit-musl` and `toolchains/dunit-tcc`.
Each fork records its upstream base in `UPSTREAM.md`, plan in `PORTING.md`
and maintained delta in `DELTA.md`.

Green Tea Kernel is Rust `no_std`, built as `libkernel.a` and linked with the
C/NASM HAL. Limine supplies BIOS/UEFI boot. HAL owns hardware entry, GDT/TSS/IDT,
syscall entry/return, context-switch assembly and port I/O. The kernel provides
VM, processes/threads, scheduling, VFS/storage, IPC, handles and device mechanisms.
Independent Rust/C ELF applications use the generated native Dunit ABI through
`libdunit`, the SDK or Dunit musl. POSIX provides source portability; the kernel
does not implement the Linux ABI.

Desktop policy belongs in userspace. `gui_server` owns display/input master
handles and hosts the compositor and DWM shell; retaining the shell there was
the accepted M7.9 decision. `init` (PID 1) supervises manifest-defined services.
Apps keep separate address spaces and crash isolation. The kernel must not know
application names, IDs, paths, icons, themes, panels, docks or launcher entries.
Apps/assets come from initrd/filesystem, never kernel `include_bytes!`.

The application scheduler is still UP: AP probes and parked CPUs do not imply
simultaneous application execution. Root is MemFS; DunitFS mounts at `/persist`.
Signals, process-image exec/fork, dynamic linking/TLS, networking, audio,
complete USB and advanced power services have open gates. Check the
[technical roadmap](../../DUNIT_OS_TECHNICAL_ROADMAP.md) and
[performance roadmap](../../DUNIT_OS_PERFORMANCE_ROADMAP.md) before making claims.

## Autonomy and approval boundaries

Complete authorized work without handing routine commands, debugging, test runs
or repair to the user. Investigate failures, fix their causes and rerun relevant
checks. Choose routine implementation details independently.

Obtain a human decision before an unapproved change to these contracts:

- New syscalls or changes to `abi/*.abi`, including numbers and rights.
- Public `gui-v1` wire format or binary userspace compatibility.
- Kernel/userspace responsibilities, privileges, capabilities or security policy.
- Initrd or on-disk filesystem formats.
- Dangerous actions listed below.

Explain concrete alternatives and a recommendation while continuing independent
work. Existing user authorization covers its stated scope; a roadmap entry alone
does not approve future contract changes.

## Implementation quality

Solve problems in the layer that owns them. Do not add fake-success stubs,
unnecessary workarounds, duplicate logic, required-work TODOs, dead code,
commented-out blocks or permanent debug output. Unsupported functionality returns
documented `ENOSYS`/`ENOTSUP`, never silently ignores parameters or claims success.
An unavoidable temporary workaround must be minimal, record its blocking reason
and removal condition in code, avoid a second source of truth and appear as
technical debt in the final report.

Read each file's current contents before editing. Before substantial work, read
the roadmap, inspect affected modules and all callers, tests and boundaries.
Resolve code/documentation contradictions using evidence. Search all enum
consumers when adding a variant; do not hide missing handling with a wildcard.

Keep responsibilities clear and files manageable. Split mixed responsibilities
or files approaching roughly 1,500 lines before expanding them. Avoid speculative
frameworks and layers. Before removing a subsystem, review its contents and
history, move reusable behavior to its owner, then remove dependencies, imports,
initialization, state, tests, build entries, assets and documentation references.
Search the entire repository for old symbols/paths and verify the result.
Preserve existing user changes and other developers' work.

## ABI and userspace

`abi/syscalls.abi`, `abi/errno.abi` and `abi/rights.abi` are authoritative.
`tools/gen_abi.py` generates Rust/C definitions and checks the kernel enum;
`--check` is a CI gate. Never edit generated files or add a syscall only to the
kernel enum. After approval, fetch and check the latest manifest and enum before
choosing a number; regenerate every output and run conformance checks.
The convention is `rax` number/result, `rdi/rsi/rdx/r10/r8/r9` arguments,
with negative POSIX error returns.

`libdunit` provides low-level native wrappers; kernel/ABI tests stay on that
layer. Apps use available SDK functionality. Service migrations use lower SDK
layers rather than rewriting the compositor or service manager as widgets.
Protect dangerous device/process operations with capabilities immediately.
Keep public wire structures separate from kernel-private and musl structures.

## Configuration and GUI

One authoritative configuration source controls changeable desktop policy:
application metadata, themes/fonts/icons, wallpaper, panels/docks/windows,
widgets, workspaces, shortcuts, rules and animations. Replacing a hardcoded match
with a map of constants is not configuration. Runtime must consume configuration;
verify changes through to visible behavior.

Technical invariants stay in code: ABI layouts, pixel-format arithmetic,
algorithm constants, memory-safety/resource limits and hardware constraints.
Reload follows `read -> parse -> validate -> resolve -> apply` atomically.
A malformed candidate keeps the previous working state and clear diagnostics.
Shared appearance tokens remain authoritative. Preserve GUI boot, input/display,
multiple clients, focus/move/resize/close, Terminal/Files, themes/wallpaper,
panels/dock/workspaces, transparency and blur. Organize `gui_server` by compositor,
protocol, windows, input and configuration ownership. Never restore the removed
kernel desktop.

## musl and TinyCC

Target `x86_64-dunit`, static first. `tools/dunit/build-libc.sh <sysroot>` builds
`libc.a`, crt objects and `dunit-cc` in `build/dunit-sysroot`; the driver emits
static `ET_EXEC`. Regenerate the musl mapping through `gen_syscall_h.py`, never
hand-edit `arch/x86_64/bits/syscall.h`. After fork source changes, invalidate the
generated `build/dunit-sysroot` and `build/dunit-musl-build` caches so tests cannot
use old libc; retain fingerprint checks. New musl tests belong in
`userspace/ctests`, the normal Makefile build and `tools/m6_*_markers.json`, and
run with `exec <name>`.

Keep complete upstream trees and narrow native adapters. Commit/test/fetch/rebase
and push the fork first; only then commit the OS submodule pointer separately.
Never publish a pointer to a fork commit absent from its remote repository.

## Verification

All OS builds, boots and runtime verification use `tools/qemu_test.py`.
Do not run QEMU manually or create a competing runner. A quick kernel compile
check is allowed before an expensive harness build, using exact Makefile flags
(`-Zbuild-std`, `compiler-builtins-mem`, `json-target-spec`). Independent host/unit
checks are allowed. Compilation alone proves no runtime claim.

```bash
python3 tools/qemu_test.py --build --markers-file tools/<suite>_markers.json \
  --post-boot-delay 3 --settle 4 --cmd-timeout 45 --timeout 220 --boot-timeout 120
```

`--build` creates HAL, kernel, userspace and ISO; compatible follow-up checks
can reuse `build/microkernel.iso`. Increase loaded-runner timeouts when needed.
Use deterministic Terminal/GUI configs, never an interactive menu. The harness
must complete and shut down QEMU without user participation.

Marker contracts are `tools/*_markers.json`: sequential `run.cmd`, required and
forbidden markers with claims/source references. A VM-stopping command such as
poweroff/reboot must be the only command in its run. Matching uses substrings
across serial output: each marker fits one line without an embedded newline.
Verify observed behavior, not a message inserted solely to pass a test. The
mandatory marker baseline requires `[TERM-007] Starting main loop` and forbids
`[PANIC]`; preserve the existing boot/readiness contracts.

Never weaken, suppress or rewrite tests/warnings/markers to pass. Removing tests
is allowed only with removal of their subsystem, and must be reported. Inspect
`build/qemu-serial.full.log` after each run for boot readiness, required/forbidden
markers, exit statuses, faults and resource cleanup. `[USER-FAULT]` is expected
only in a test that checks it; unsupported calls returning documented `ENOSYS`
are not false success. Diagnose failures, rerun after repair and report relevant
serial evidence. Reading contracts or historical results is not a fresh test.

## Shared workspace and Git

Delegated agents may research read-only or edit disjoint assigned files. Builds
and QEMU sharing `build/`, ISO or QMP socket must be serialized. Parallel builds
require isolated worktrees and build directories. The Git index, commits and
submodule updates are shared mutations: serialize them. Every agent follows
this contract and respects existing changes.

After relevant checks pass, complete the authorized commit/push workflow:

1. `git fetch origin`; compare `HEAD...origin/<branch>` using
   `git rev-list --left-right --count`.
2. Rebase on `origin/<branch>` if behind. Resolve conflicts preserving both
   developers' intent; rerun affected checks after incoming changes.
3. Stage explicit paths with `git add <path>`, never `git add -A`.
4. Make small thematic commits, imperative `<area>: <action>`, preserving
   established `Co-Authored-By` trailers.
5. `git push origin <branch>`. A non-fast-forward rejection requires another
   fetch/rebase/test cycle, never force push.

Do not publish unverified code or red tests. Do not attribute another developer's
commits to yourself or the user or revert them without an explicit decision.
Exclude build products, logs, images, backups (`*.bak`, `*.orig`) and temporary
files from commits.

## Dangerous actions

Explicit human authorization is required for physical disk or host `/dev/*`
writes; real-device mkfs/install/blkwrite; force push, hard reset, forced Git
clean or branch deletion; Git configuration changes; deleting pre-existing
files without reviewing contents and history; publishing code/secrets externally
outside the authorized workflow; or bypassing fetch/rebase checks. Disposable
QEMU images in `build/*.img` and generated-cache invalidation are normal
verification scope.

## Final audit and report

Audit boundaries, old symbols/paths, policy hardcoding, duplicate sources,
resource lifetimes and preserved behavior. Exclude artifacts and backups.
Report changes/decisions, removed legacy/hardcoding, intentional limits, tests
and serial evidence, commit/push hashes and branch, and remaining debt. For fork
changes report both fork commit and OS pointer update. Keep the report concise
and use the user's requested language; repository Markdown is English.
State unverified work and limitations plainly.
