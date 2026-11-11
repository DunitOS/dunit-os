# OP1/OP2 — CPU-local entry and isolated AP hardware bring-up

11 November 2026. [Measured matrix](smp-bringup.json) covers BIOS 1/2/4/8
vCPUs and UEFI 4 vCPUs on QEMU TCG. Every AP actually executes its own checked
65,536-iteration sum, receives local timer interrupts and completes a two-way
IPI round trip. This is hardware bring-up, **not a parallel process scheduler**.

## Implementation and ownership

Limine MP publishes AP entry points after the BSP has prepared all private CPU
storage and shared kernel page tables. Each AP switches to the kernel CR3 and
its own stack, initializes x87/SSE, GS, GDT/TSS/IST, IDT register, syscall MSRs
and local APIC. The read-only IDT is shared. Logical CPU indices are distinct
from hardware APIC IDs. The current capacity is 64; larger configurations fail
explicitly. BSP resources are static, while stacks/descriptors for APs are
allocated only for the CPUs reported by Limine, before AP publication.

One schema generates C/NASM CPU-local offsets and Rust capacity, with C layout
assertions and a build freshness check. Syscall scratch, stack/escape pointers,
user FPU pointer and current-task/exit/yield/scheduling flags are CPU-local.
Kernel GS is restored on syscall and paranoid interrupt entry, including NMI
transition windows; user transitions restore user GS. FSGSBASE is disabled so
userspace cannot forge that anchor. Existing FS/TLS and x87/SSE context handling
are retained. XSAVE/AVX context support is outside this change.

The BSP retains PIC device routing through local APIC virtual wire and PIT
clock/preemption. APs handle only private APIC probes; they never enter process,
VFS, heap or userspace paths. Timers are calibrated against PIT edges, then
masked once progress is proven. Parked APs use HLT and remain IPI-wakeable;
there is no permanent periodic wakeup tax. High-resolution clocksource and
cross-CPU TSC validation remain open. xAPIC is tested; the x2APIC path is not
claimed validated by these runs.

IPI probes run one round trip at a time. APIC IRR coalesces identical pending
vectors, so treating a simultaneous burst as a reliable message count was
incorrect. Request/ack state is published with release/acquire ordering and
checked against both destination acknowledgement and BSP reply count.

## ELF and memory footprint

The link now collects unreachable sections while preserving KEEP roots. GOT
is explicitly placed before NOLOAD BSS: previously an orphan GOT could make a
LOAD segment store the entire zero-fill hole in the ELF file. The layout check
rejects file-backed BSS and invalid LOAD sizes on every link. Negative checks
confirmed rejection of both a PROGBITS BSS and a LOAD covering the zero-fill.

For the tested full terminal image, the kernel ELF is 3,603,768 bytes; BSS is
2,539,672 bytes and its data LOAD has 455,336 file bytes / 2,998,424 memory bytes.
The initial OP0 fixture image had a 6,129,736-byte kernel after section GC.
These are different builds: this is a footprint observation, not a runtime
speedup measurement. No applications were removed and the ESP was not enlarged.

## Reproduction and regression results

```sh
python3 tools/test_smp_boot.py
python3 tools/qemu_test.py --uefi --cpus 4 --accel tcg \
  --markers-file build/performance/smp-boot/4/markers.json \
  --workdir build/performance/smp-uefi4 --boot-timeout 120 \
  --timeout 360 --cmd-timeout 180
python3 tools/qemu_test.py --cpus 8 --accel tcg \
  --markers-file tools/m6_thread_stress_markers.json \
  --timeout 360 --cmd-timeout 240
python3 tools/qemu_test.py --cpus 8 --accel tcg \
  --markers-file tools/c_pointer_markers.json --timeout 420 --cmd-timeout 240
python3 tools/test_c_pointer.py --verify-only
```

The hardware matrix retains full boot/DF/IRQ assertions and runs the native TCC
CPU/pthread, allocator and MemFS correctness benchmark in every VM. Required AP
rows must have unique logical/APIC IDs, ready state, checked work, actual IPI
progress and at least two local timer ticks. Host negative tests reject fabricated
online/progress data. The scheduler reports `dispatch_cpus=1` separately.

Additional regressions: 8-vCPU native thread stress passed 1,000 joinable TLS/errno/
allocation cycles and 1,000 detached auto-reaps; C pointer modes passed the full
marker budget and exact cursor hide/restore pixel assertions. The 4-vCPU GUI
smoke retained all 30 required markers; its terminal-image adapter moves the
original actions into `command_actions` for `exec /app/gui_server`, preserving
their original marker budget. Host QEMU tests (19), measurement tests (4), AP
negative test (six rejection cases), ABI and generated-layout checks passed.

## Remaining safety gates

[Shared-state audit](smp-audit.md) identifies unsafe process-table references,
VFS/backend raw-pointer lifetimes, wait/wakeup transactions, lock order and remote
TLB invalidation. These must be corrected before AP userspace dispatch. AP stacks
are permanent because CPU hotplug is outside this milestone. No new public ABI,
GUI wire format, filesystem format or privilege mechanism is introduced.

OP0 is partial; OP1 ownership and OP2 clock work remain open. OP3/OP4 and the
performance stages are not marked complete. There is no application scaling
claim and no KVM performance result: this runner has no usable `/dev/kvm`.
