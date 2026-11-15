# Welcome to the Dunit OS Wiki

**Dunit OS** (codename **Green Tea**) is an experimental, modern 64-bit operating system designed as an honest, practical, and highly capable platform for low-level systems programming and lightweight desktop computing. The project pairs an x86_64 `no_std` Rust kernel with a minimal C/NASM Hardware Abstraction Layer (HAL), Limine boot protocol, POSIX-aligned userspace ABI, musl libc integration, a compositing window manager (DWM v2), and a native component-based graphical user interface toolkit.

Unlike monolithic systems or academic toy kernels, Dunit OS is built around **honest subsystems** and **observable execution**: components do not fake success, errors propagate truthfully, and every architectural milestone is validated against automated test harnesses in QEMU.

---

## Navigation Portals

The documentation is organized into clear functional portals modeled after the ArchWiki structure:

### 1. System Architecture & Internals
* **[Architecture](Architecture)** — Complete system layering, Limine boot handoff, C/NASM HAL, Rust `no_std` microkernel, memory management (PMM, VMM, Kernel Heap), preemptive scheduler, threading model, and Inter-Process Communication (IPC).
* **[Kernel & Syscalls](Kernel-and-Syscalls)** — The x86_64 `syscall` calling convention, full catalog of syscalls (0 through 40), error code definitions (`errno`), memory protection, and process control.
* **[Hardware & Drivers](Hardware-and-Drivers)** — CPU setup, interrupt controllers (PIC/IOAPIC, IDT), 8042 PS/2 keyboard and mouse, Bochs VBE and VGA display adapters, AHCI/SATA storage, and Intel e1000 PCI network interface detection.
* **[Storage & Filesystems](Storage-and-Filesystems)** — Virtual File System (VFS) architecture, standard file descriptors, in-memory MemFS root, Initrd packaging, and persistent DunitFS v2 on-disk filesystem.

### 2. User Environment & Desktop
* **[DWM Window Manager](DWM-Window-Manager)** — DWM v2 userspace compositor, declarative multi-panel layout configuration (`schema_version = 2`), widget architecture, Dunit Style Sheets (DSS), Lua animation & event scripting engine, and UNIX domain socket IPC.
* **[GUI Server & UI Kit](GUI-Server-and-UI-Kit)** — System UI server (`gui_server`), component tree architecture, built-in widgets (buttons, textboxes, lists, trees, sliders), theming engine, drag-and-drop, and window management protocols.

### 3. Development & Porting
* **[SDK & Development](SDK-and-Development)** — Native Rust application development using `cargo-dunit`, project manifests (`dunit.toml`), C application development with `dunit-cc` and musl libc, and the `<dunit/ui.h>` component SDK.
* **[Porting Applications](Porting-Applications)** — Practical guidelines and lessons learned from porting complex C/Rust software: Doom, Quake, Minecraft C Edition, Game Boy (gnuboy), GrafX2, Lite XL, Video Player, and FastTracker/ProTracker audio player.
* **[Installation & Building](Installation-and-Building)** — Host prerequisites, building bootable ISO images (`make iso`, `build_iso.sh`, `build_bundle_iso.sh`), QEMU virtual machine parameters, disk installation, and automated test runners.

### 4. Maintenance & Support
* **[Troubleshooting](Troubleshooting)** — Diagnosing kernel panics, decoding page faults (`CR2`), analyzing serial console logs, resolving memory exhaustion in 64-bit ports, handling mouse input edge cases, and debugging SDK link errors.

---

## The Dunit Way

The design and evolution of Dunit OS adhere to four fundamental principles:

1. **Honest Subsystems**: Subsystems report their real state. If a feature or syscall is unimplemented, it returns `ENOSYS`, `EAGAIN`, or `EOPNOTSUPP` rather than pretending to succeed.
2. **Strict Layering**: Clear boundaries separate bootloader (Limine), hardware abstraction (C/NASM HAL), microkernel (Rust `no_std`), system services (`gui_server`, `vfs`, `devfs`), and userspace applications.
3. **Deterministic Verification**: Every milestone and bugfix is proven through observable serial logs and automated test harnesses (`tools/qemu_test.py`, `boot_markers.json`).
4. **Lean & Purposeful Runtime**: Zero unnecessary abstractions. Userspace binaries are statically linked ELF executables using tailored runtimes (`libdunit`, `dunit-musl`).

---

## Architectural Snapshot

```text
+--------------------------------------------------------------------------+
|                        Userspace Applications                            |
|  Doom · Quake · Minecraft C Edition · GrafX2 · Lite XL · Tracker · Calc  |
+------------------------------------+-------------------------------------+
|         Rust Native Apps           |           C / C++ Apps              |
|        (libdunit + dunit)          |       (dunit-musl + dunit-cc)       |
+------------------------------------+-------------------------------------+
|                  Desktop Environment & System Services                   |
|       DWM v2 Compositor (DSS/Lua) · gui_server · VFS · Initrd · DevFS    |
+--------------------------------------------------------------------------+
|                         Dunit System Call ABI                            |
|             x86_64 `syscall` instruction · Syscalls 0..40                |
+--------------------------------------------------------------------------+
|                    Dunit OS Kernel (Rust `no_std`)                       |
|   PMM · VMM (PML4) · Heap · Preemptive Scheduler (SSE/FPU) · IPC Queues  |
+--------------------------------------------------------------------------+
|                   Hardware Abstraction Layer (HAL)                       |
|        GDT · IDT · Interrupt Handlers · Syscall Stubs · PS/2 · AHCI      |
+--------------------------------------------------------------------------+
|                        Bootloader (Limine v5)                            |
|      BIOS / UEFI Handoff · Higher-Half Direct Map (HHDM) · Framebuffer   |
+--------------------------------------------------------------------------+
```

---

## Quick Reference

| Resource | Description | Path / Command |
| --- | --- | --- |
| **Source Repository** | Main system repository | `https://github.com/DunitOS/dunit-os` |
| **Wiki Repository** | GitHub Wiki repository | `https://github.com/DunitOS/dunit-os.wiki` |
| **Build ISO** | Build bootable hybrid ISO | `make iso` or `./build_iso.sh` |
| **QEMU Launch** | Run image in QEMU with serial output | `qemu-system-x86_64 -cdrom build/dunit.iso -m 1024M -serial stdio` |
| **Rust SDK** | Create new native application | `cargo dunit new my_app --kind gui` |
| **C Compiler** | Static C compiler wrapper | `/path/to/dunit-sysroot/bin/dunit-cc -O2 main.c -o app` |
| **DWM Config** | Main desktop configuration | `/system/share/dwm/default.toml` |
