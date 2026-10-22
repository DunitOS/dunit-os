<p align="center">
  <img src="assets/images/banner.png" width="1000" alt="Dunit OS banner"/>
</p>

Dunit OS
========

Dunit OS is an x86_64 operating system written from scratch with a Rust `no_std` kernel (**Green Tea Kernel**), a C and Assembly hardware abstraction layer, and a userspace desktop environment.

The system is designed with a strict mechanism-policy split: the kernel provides fundamental services (memory management, scheduling, drivers, and process isolation), while desktop policy, window management, and applications run as independent ring-3 ELF processes in userspace. A crash in a user application is isolated and does not affect the rest of the system.

Bootloader and Modes
--------------------

### Limine Bootloader
<p align="center">
  <img src="assets/images/bootloader.gif" width="100%" alt="Limine Bootloader"/>
</p>

### Terminal Mode
<p align="center">
  <img src="assets/images/terminalmode.gif" width="100%" alt="Terminal Mode"/>
</p>

### DWM Desktop Mode
<p align="center">
  <img src="assets/images/dwm.gif" width="100%" alt="DWM Desktop"/>
</p>

Quick Start
===========

Prerequisites
-------------
A standard 64-bit Linux development environment:
`qemu-system-x86_64`, `gcc`, `nasm`, `lld`, `xorriso`, `python3`, and a nightly Rust toolchain with `rust-src` (`rustup override set nightly && rustup component add rust-src`).

Build and Run
-------------
Clone the repository and build the bootable ISO:

```bash
git clone https://github.com/DunitOS/dunit-os.git
cd dunit-os
./build_iso.sh
```

Launch in QEMU:

```bash
make run        # Boot into Terminal mode
make run-dwm    # Boot into DWM Desktop mode
```

Common build targets:  
`make iso` — Builds the terminal ISO (`build/microkernel.iso`)  
`make iso-dwm` — Builds the graphical desktop ISO  
`make disk-image` — Creates a bootable BIOS/UEFI raw disk image  

System Overview
===============

Architecture and Kernel
-----------------------
**Boot Flow**: Limine bootloader with both BIOS and UEFI support.  
**Kernel**: Rust `no_std` microkernel architecture (Green Tea Kernel).  
**HAL**: C/NASM layer managing GDT, IDT, interrupts, context switching, and syscall entry.  
**Process Model**: Independent ring-3 ELF processes with parent/child tracking and clean termination on faults.  

Multitasking and Memory
-----------------------
**Preemptive Scheduler**: Timer-driven (PIT) round-robin scheduler preserving FPU and SSE registers across switches.  
**Threading**: Multi-threaded processes with kernel-level thread management and `pthread` support.  
**Virtual Memory**: PMM and VMM allocators, per-process page tables, guard pages, `mmap`/`munmap`/`mprotect`, and W^X enforcement.  

Terminal and Shell
------------------
**Console**: VGA framebuffer terminal using a Linux-style 8x16 bitmap console font.  
**Shell (dsh)**: Command history (Up/Down arrows), tab autocomplete, pipes (`|`), and path execution (`/system/bin`, `/app`).  
**dufetch**: Displays hardware specifications, memory statistics, and the ASCII logo.  
**calc**: Command-line mathematical expression calculator.  
**bmp_viewer**: Renders BMP image files directly to the framebuffer.  
**fsck_dunit**: Filesystem consistency check and diagnostic utility.  

Desktop Environment (DWM)
-------------------------
**Compositor**: `gui_server` software compositor running entirely in userspace.  
**Configuration**: Panel placement, dock items, themes, wallpapers, and widgets (clock, battery, stats) configured through TOML (`default.toml`).  
**Visuals**: Per-pixel ARGB transparency, drop shadows, and responsive window dragging/resizing.  
**Window Switcher**: Alt+Tab and Super key switcher with live active window outlines.  
**Applications**: GUI Terminal, File Manager (`gui_files`), Calculator (`gui_calc`), Settings (`gui_settings`), and System Stats monitor (`gui_stat`).  

Storage and Filesystems
-----------------------
**Block Drivers**: Native AHCI SATA and VirtIO block device support with GPT partitioning.  
**Filesystems**: Ramdisk (MemFS) for `/app` and `/assets`, and a custom extent-based DunitFS.  
**Disk Installation**: The `install.dunit` command installs the operating system to physical drives or USB media, creating an EFI System Partition and a persistent `/persist` filesystem that survives reboots.  

C Runtime and Toolchain
-----------------------
**Musl Libc**: Dedicated static `musl libc` port mapped directly to the native Dunit syscall ABI.

Host cross-compiler:
```bash
build/dunit-sysroot/bin/dunit-cc -O2 -o build/userspace/hello hello.c
```

On-device compilation via Tiny C Compiler (`tcc`):
```text
exec tcc /assets/tcc/hello.c -o /tmp/hello
exec /tmp/hello
```

Current Limitations
===================

This is an experimental hobby operating system under active development. Current omissions:  
Multi-core (SMP) — execution is currently single-core (UP)  
Networking stack (TCP/IP) and audio subsystem  
Dynamic linking (`.so` shared libraries)  

License
=======

This project is licensed under the MIT License.
