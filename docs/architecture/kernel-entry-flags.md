# Direction Flag at kernel entry

Kernel C/Rust follows the x86-64 ABI requirement that DF is clear. Userspace can
legitimately hold DF temporarily inside backward `memmove`; timer interrupts can
arrive anywhere in that sequence. Hardware interrupt entry saves RFLAGS but does
not clear DF. Calling Rust with the inherited flag can make compiler-builtins
string operations copy backwards and corrupt kernel data/return frames.

A native 10,000-row SDK list workload exposed intermittent instruction page
faults at RIP/address 0. A debugger capture recorded kernel RFLAGS `0x10406`
(DF set) and a damaged scheduler return frame. No GUI/kernel boundary or user
ABI change is needed to correct the entry invariant.

- `isr_common_stub` executes CLD before calling C/Rust. The hardware-saved
  interrupted RFLAGS remain untouched for IRET, including interrupted kernel
  backward copies. The scheduler escape path also continues with DF clear.
- SYSCALL's SFMASK clears both IF and DF (`0x600`). The original user flags in
  R11 are still saved and restored through IRET. Clearing live kernel DF must
  not erase the user's saved direction flag.

The existing `preempt_child` smoke deliberately runs CPU-bound work with DF set,
checks DF and XMM state after interrupts, and makes a GetPid syscall with DF set
and verifies its result and restored user flags. Rust code is only resumed after
CLD in the inline assembly. Boot-smoke builds assert the live kernel flag at
both IRQ and syscall entry; they also record an actual DF-set user interrupt.
`run_preemption_smoke` requires that observation, successful child status, and
committed timer switches before emitting `[DF-TEST] syscall+irq flags preserved`.
Both `tools/boot_markers.json` and the native list fixture require the marker.

The list fixture additionally exercises the original allocation/copy workload,
selection of 10,000 rows, keyboard and pointer input, scrolling and resize without
a debugger. Debugger runs are diagnostic only because breakpoints change timing.
