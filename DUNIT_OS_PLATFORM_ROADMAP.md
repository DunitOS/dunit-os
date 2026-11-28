# Dunit OS Platform Evolution Roadmap (Dynamic Runtime, Performance, Storage & GPU)

This roadmap defines the OS-level platform work needed to graduate Dunit OS from a static single-CPU desktop into a general-purpose x86_64 platform capable of running large, multi-threaded, dynamically linked C/C++20 and Rust workloads — including future heavy applications such as a modern web engine — without turning this document into an application or browser-specific plan.

**No duplication rule:** existing tasks and acceptance gates stay in their owning documents and are linked rather than repeated here:
- Core OS milestones, storage (`M5`), musl/toolchain (`M6`), SDK (`M7`), networking (`M8`), apps (`M9`), and future engine selection (`M10`): [Technical Roadmap](DUNIT_OS_TECHNICAL_ROADMAP.md)
- SMP bring-up, lock/ownership gates, CPU scaling, software rendering, and I/O measurements (`OP0–OP9`): [Performance & SMP Roadmap](DUNIT_OS_PERFORMANCE_ROADMAP.md)
- Implemented behavior vs. open gates: [Current System State](docs/architecture/current-state.md)
- Native userspace ABI & musl adaptation rules: [Native ABI](abi/README.md), [Dunit musl porting](toolchains/dunit-musl/PORTING.md)

---

## Target Outcome & Ordering

A modern native workload requires four OS capabilities working together:
1. **Dynamic Linking & C++20 Toolchain (`P0`)** — shared `.so` loading, dynamic TLS, `libc++`, and missing POSIX primitives.
2. **SMP & Software Performance (`P1`)** — closing `OP3` safety gates and unlocking multi-CPU execution, cache locality, and low IPC/compositor latency via [OP0–OP9](DUNIT_OS_PERFORMANCE_ROADMAP.md).
3. **Demand-Paged Storage & Large-Binary Execution (`P2`)** — running apps and shared libraries directly from persistent disk (`M5`) with shared read-only text pages instead of keeping everything duplicated in RAM/initrd.
4. **GPU Display Pipeline (`P3`)** — `VirtIO-GPU` 2D DMA scanout, hardware cursor, zero-copy client promotion, and bounded acceleration on top of the mandatory software/Bochs fallback.

Supporting desktop/platform services (`P4` — entropy, audio, font discovery, crash telemetry) run alongside `P0–P3` once ownership contracts are approved per [contributor boundaries](.github/contributors/CLAUDE.md).

```text
  [P1: OP3 Gates + OP4/OP5 SMP + OP6/OP8 Latency/IO] ──┐
  [P0: Dynamic ELF/ld.so + C++20 libc++ + POSIX]     ──┼──► Heavy Native Platform
  [P2: M5 Disk-Backed Root + Shared Page Cache/VM]   ──┤    (C++20/Skia/Media/Apps)
  [P3: VirtIO-GPU 2D DMA + Hardware Cursor + Fences] ──┘
```

---

## P0 — Dynamic Linking, Runtime ABI & C++20 Toolchain

**Builds on:** `PT_INTERP` handoff and single-`DT_NEEDED` relocation proof already verified in [M6 — native static musl and C toolchain](DUNIT_OS_TECHNICAL_ROADMAP.md#m6--native-static-musl-and-c-toolchain).

### P0.1 Complete Userspace Dynamic Linker (`musl ldso` on `x86_64-dunit`)
- [x] Kernel VM/mapping and ELF execution prerequisites for `ld.so`: `MAP_FIXED` anonymous & `MAP_PRIVATE | MAP_FIXED` file mappings (preserving guard/shared regions while replacing private ranges cleanly), over-EOF DSO virtual-span `sys_vm_map_file` (`W^X` `protect_range` enforcement), and general VFS `PT_INTERP` (`ET_EXEC` & `ET_DYN` PIE + interpreter) loading in `exec`/`spawn` (`kernel/src/process/mod.rs`, `kernel/src/syscall/mod.rs`, `kernel/src/syscall/native_fd.rs`, `kernel/src/elf/`).
- [ ] Build & package `musl ld.so` / shared `.so` toolchain sysroot support and enable full dynamic relocation & symbol resolution: `R_X86_64_RELATIVE`, `GLOB_DAT`, `JUMP_SLOT`, `COPY`, `64`, `DT_NEEDED` transitive graph, `DT_RUNPATH`/`RPATH`, `/system/lib` lookup, `GNU_HASH`/`SYSV_HASH`.
- [ ] Enforce `PT_GNU_RELRO` (read-only after relocation) and `.init_array`/`.fini_array` execution order.
- [ ] Support `dlopen` / `dlsym` / `dlclose` and dynamic per-module TLS (`DTV` + `__tls_get_addr` / `TLSDESC`) across multi-threaded processes.
- [x] Keep static `ET_EXEC` verification and W^X enforcement intact alongside dynamic `ET_DYN` gates.

### P0.2 C++20 / C++23 Cross-Toolchain (`x86_64-dunit`)
- [ ] Stand up LLVM/Clang + LLD sysroot profile targeting `x86_64-dunit` (no host header/library leakage).
- [ ] Port and build `compiler-rt` builtins, `libunwind` (DWARF/eh_frame unwinding), `libc++abi`, and `libc++` (both `libc++.a` and `libc++.so`) on top of `dunit-musl`.
- [ ] Provide CMake / Meson / Ninja toolchain files for third-party C++ libraries (filesystem, chrono, std::thread, mutex/condition_variable, atomic wait/notify via Dunit futex).
- [ ] Add automated C++20 smoke tests in `tools/qemu_test.py` covering exceptions, RTTI, `std::thread`, `std::filesystem`, and shared library loading (`dlopen` + C++ static constructors).

### P0.3 Missing POSIX Runtime Foundations
*(See open contracts in [Current System State](docs/architecture/current-state.md#open-gates) and [M6](DUNIT_OS_TECHNICAL_ROADMAP.md#m6--native-static-musl-and-c-toolchain).)*
- [x] **Kernel CSPRNG + Hardware Entropy**: ChaCha20 pool fed by `RDSEED`/`RDRAND`, timing jitter (`kernel/src/entropy.rs`), and quarantined legacy PCI `VirtIO-RNG` (`kernel/src/drivers/virtio_rng.rs`).
- [ ] **Userspace CSPRNG Parity**: `/dev/urandom`, `getentropy`/`getrandom` ABI/musl coverage across dynamic processes (beyond boot `AT_RANDOM`).
- [ ] **POSIX Signals & Fault Semantics**: signal frame delivery/return on user stack, `sigaction` (`SA_SIGINFO`, `SA_RESTART`), signal masks, `sigaltstack`, and thread-targeted signals (`pthread_kill`) with defined syscall interruption semantics.
- [ ] **Multi-process Handle Passing & Spawn**: capability/fd transfer across IPC (equivalent to `SCM_RIGHTS` / descriptor passing over Dunit IPC) and `execve` / `posix_spawn` hardening for multi-process sandboxes.
- [ ] **Non-blocking & Poll Parity**: complete `epoll`/high-density `poll`/`wait_set` scaling for event-driven runtimes without per-call linear scans.

**Acceptance:** A multi-threaded dynamically linked C++20 binary using `libc++.so`, exceptions, `dlopen`, `std::thread`, `getrandom`, signals, and cross-process fd handoff passes native QEMU tests under W^X.

---

## P1 — SMP, Scheduler & Core Performance Completion

This platform track **does not duplicate** `OP0–OP9`; instead it tracks the exact dependencies required from the [Performance & SMP Roadmap](DUNIT_OS_PERFORMANCE_ROADMAP.md):

1. **Telemetry & Reference Hardware Gate** — close KVM instrumentation, tail latency (`p95`/`p99`), and RSS/CPU accounting in [OP0](DUNIT_OS_PERFORMANCE_ROADMAP.md#op0--reproducible-baseline-and-instrumentation).
2. **Pre-SMP Safety Gates** — complete IRQ-off bounds, lock ordering, remote cancellation/stop acknowledgments, and concurrent VM/DMA stress in [OP3](DUNIT_OS_PERFORMANCE_ROADMAP.md#op3--memory-ownership-and-synchronization). *(Active-CPU remote cancel metadata, group-entry revocation, and IPI reschedule poke landed in `kernel/src/process/execution_state.rs`, `execution.rs`, `syscall_boundary.rs`, and `lifecycle.rs`; remaining OP3 gates stay tracked in `DUNIT_OS_PERFORMANCE_ROADMAP.md`.)*
3. **True Multi-Core Execution** — enable AP userspace dispatch (`dispatch_cpus > 1`), per-CPU runqueues/accounting, and interactive scheduling without starving heavy workers in [OP4](DUNIT_OS_PERFORMANCE_ROADMAP.md#op4--concurrent-processthread-scheduling) and [OP5](DUNIT_OS_PERFORMANCE_ROADMAP.md#op5--scheduler-scaling-and-predictable-latency).
4. **Pipeline Latency & Software Fast Paths** — eliminate avoidable IPC/copy stalls, optimize software blits/AVX/parallel raster tiles, and add per-CPU slab/allocator + VFS caching in [OP6](DUNIT_OS_PERFORMANCE_ROADMAP.md#op6--input-ipc-and-sdk-latency), [OP7](DUNIT_OS_PERFORMANCE_ROADMAP.md#op7--software-rendering-and-parallel-sdk-work), and [OP8](DUNIT_OS_PERFORMANCE_ROADMAP.md#op8--memory-files-and-io).

**Acceptance:** Verified multi-CPU scaling under KVM (`>= 2.5x` on 4 vCPUs on parallel CPU workers) and loaded desktop `p95 <= 33 ms` per [OP0/OP9](DUNIT_OS_PERFORMANCE_ROADMAP.md#op9--final-matrix-and-regression-protection).

---

## P2 — Installed Disk Root, Demand Paging & Memory Scalability

**Builds on:** [M5 — installed root, provisioning and recoverable storage](DUNIT_OS_TECHNICAL_ROADMAP.md#m5--installed-root-provisioning-and-recoverable-storage) and [OP8 — memory, files and I/O](DUNIT_OS_PERFORMANCE_ROADMAP.md#op8--memory-files-and-io).

Large native software and shared libraries (`10–150+ MB` binaries/assets) cannot live inside an in-RAM `initrd.img` on a 512 MiB target.

- [ ] **Boot & Exec from Disk (`M5` completion)**: complete [M5](DUNIT_OS_TECHNICAL_ROADMAP.md#m5--installed-root-provisioning-and-recoverable-storage) disk-root boot so `/system/{bin,lib,share,services}` and `/apps/<id>` load from AHCI/VirtIO-blk DunitFS instead of copying everything into initrd MemFS.
- [ ] **Shared Page Cache & Demand-Paged Read-Only `mmap`**: replace eager copy-all snapshots for read-only `.text`/`.rodata`/DSO segments and font/locale asset files (`MAP_SHARED` read-only / demand-paged `MAP_PRIVATE` on page fault) so multiple processes share physical pages cleanly.
- [ ] **Large-Address-Space & OOM Policy**: tune PMM/VMM and page-table reclamation for multi-hundred-MiB process heaps, support `>= 1–2 GiB` QEMU/hardware RAM profiles cleanly, and add deterministic per-process OOM recovery rather than kernel panic.
- [ ] **Block I/O Throughput**: bounded multi-sector coalescing and read-ahead in the AHCI/VirtIO block path without weakening [OP3](DUNIT_OS_PERFORMANCE_ROADMAP.md#op3--memory-ownership-and-synchronization) DMA quarantine invariants.

**Acceptance:** Large multi-MiB C++/Rust binaries and shared libraries launch from DunitFS without inflating `initrd.img`, share read-only code pages across processes, and survive allocation pressure cleanly.

---

## P3 — GPU & Hardware-Accelerated Display Pipeline

**Extends:** display lease ownership in [OP3](DUNIT_OS_PERFORMANCE_ROADMAP.md#op3--memory-ownership-and-synchronization), [OP9 GPU escalation trigger](DUNIT_OS_PERFORMANCE_ROADMAP.md#op9--final-matrix-and-regression-protection), and [desktop architecture](docs/architecture/desktop.md).

Even fast software blits hit MMIO/CPU memory-bandwidth limits on high-DPI full-screen composition and rich media scrolling. The software `bochs-vbe`/Limine GOP path must remain functional as the baseline fallback.

### G0 — Display Backend Trait & Multi-Buffer Scanout
- [x] Separate hardware display mechanism cleanly behind a kernel `DisplayBackend` scanout trait (`bochs-vbe` / Limine GOP) and propagate clipped dirty rectangles (`sys_fb_present` -> `driver::present`), preserving existing single-owner display leases (`gui_server`).
- [ ] Support double/triple-buffered scanout page flips and vsync/present completion events (`wait_set` integration) instead of synchronous CPU MMIO stalls.

### G1 — `VirtIO-GPU` 2D Driver & Zero-Copy DMA Scanout
- [ ] PCI probe (`0x1AF4:0x1050`) and VirtIO-GPU `controlq` / `cursorq` setup using the quarantined DMA model from [OP3](DUNIT_OS_PERFORMANCE_ROADMAP.md#op3--memory-ownership-and-synchronization). *(Requires host QEMU built with `virtio-gpu-pci`/`virtio-vga` for live verification.)*
- [ ] Implement core 2D commands: `GET_DISPLAY_INFO`, `RESOURCE_CREATE_2D`, `RESOURCE_ATTACH_BACKING` (pin userspace/compositor backing pages directly for DMA — eliminating CPU MMIO `write_volatile` copies), `SET_SCANOUT`, `TRANSFER_TO_HOST_2D` (dirty sub-rectangles only), and `RESOURCE_FLUSH`.
- [ ] Implement **`cursorq` hardware cursor** (`UPDATE_CURSOR` / `MOVE_CURSOR`): pointer motion updates scanout directly in hardware with zero compositor frame repaint or pixel readback.

### G2 — Compositor & `gui-v1` Direct-Scanout / Damage Fast Path
- [ ] Extend `gui-v1` / `gui_server` client commit semantics (within [protocol rules](.github/contributors/CLAUDE.md)) for explicit client dirty rectangles (`window.present_damage`), bounded multi-buffer swapchains, and async release fences ([OP6](DUNIT_OS_PERFORMANCE_ROADMAP.md#op6--input-ipc-and-sdk-latency)).
- [ ] Enable **direct scanout promotion** when a top opaque fullscreen window matches the display geometry/format (zero intermediate compositor copy).
- [ ] Keep software `dunit-render` + SIMD (`SSE2`/optional `AVX` per [OP7](DUNIT_OS_PERFORMANCE_ROADMAP.md#op7--software-rendering-and-parallel-sdk-work)) as the automatic fallback when overlays/blur/translucency or non-GPU drivers are active.

### G3 — 3D / GPU Command Submission Boundary *(Post-G2 / Optional)*
- [ ] Evaluate VirGL / Venus (`VirtIO-GPU` 3D capsets) or discrete Intel/AMD/Mesa interface only after `G0–G2`, `P0`, and `P1` are stable.
- [ ] Route GPU command/fence submission through a capability-isolated userspace GPU service (mirroring the `netd`/`browser_netd` privilege separation) so untrusted apps never hold raw hardware DMA privileges.

**Acceptance:** Fullscreen 1920x1080 animation, scrolling, and pointer motion run on `VirtIO-GPU` with hardware cursor (`0` compositor frames on pure mouse move), zero MMIO pixel loops, and exact fallback parity on `bochs-vbe`.

---

## P4 — Supporting Rich-Application Platform Services

**References:** [M7](DUNIT_OS_TECHNICAL_ROADMAP.md#m7--native-application-sdk), [M8](DUNIT_OS_TECHNICAL_ROADMAP.md#m8--networking-and-platform-services), [remaining engineering work](DUNIT_OS_TECHNICAL_ROADMAP.md#images-release-and-remaining-engineering-work).

- [ ] **Text Shaping & Font Discovery**: system font catalog (`/system/share/fonts` / Fontconfig-compatible lookup for C++ apps) and complex-script shaping/emoji/bidi support (complementing `dunit-text` and [SDK localization goals](DUNIT_OS_TECHNICAL_ROADMAP.md#images-release-and-remaining-engineering-work)).
- [ ] **Audio Service**: kernel `virtio-snd` / HDA PCM driver + isolated userspace audio mixer service (`audiod` with capability leases, matching `netd` design).
- [ ] **Rich Clipboard & File Dialogs**: extend `dunit_transfer` with MIME-typed payload negotiation (`text/html`, `image/png`, file descriptors) for drag-and-drop and copy/paste across native and ported C++ apps.
- [ ] **Developer Diagnostics**: GDB remote stub, userspace symbolicated crash dumps / backtraces (`libunwind` integration), and bounded persistent logs per [remaining engineering work](DUNIT_OS_TECHNICAL_ROADMAP.md#images-release-and-remaining-engineering-work).

---

## Recommended Execution Sequence

| Phase | Track | Unlocks |
|---|---|---|
| **1** | `P1` (`OP3` gates → `OP4/OP5` SMP) + `P3.G0/G1` (`VirtIO-GPU` 2D + hw cursor) | True multi-core OS + zero-MMIO display + smooth UI under load |
| **2** | `P2` (`M5` disk root + demand-paged DSO cache) + `P0.1` (`ld.so` full dynamic linking) | Loading large binaries/libraries from disk without bloating RAM/initrd |
| **3** | `P0.2` (`Clang++/libc++` for `x86_64-dunit`) + `P0.3` (CSPRNG, signals, fd-passing) | Compiling real C++20/Skia/ICU/HarfBuzz codebases cleanly |
| **4** | `P3.G2` (direct scanout / explicit damage) + `P4` (fonts, audio, rich clipboard) | Full desktop parity ready to host a modern browser engine ([M10](DUNIT_OS_TECHNICAL_ROADMAP.md#m10--browser)) cleanly on top of `browser_netd` |
