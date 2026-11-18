# Framebuffer mapping scopes and interrupt progress

`fb_present` previously held the address-space IRQ-safe guard across validation,
every source read, every volatile VRAM write and backend publication. A full
1920×1080 image therefore kept its entire 8,294,400-byte transfer under the VM
pin. Enabling interrupts at syscall entry did not shorten that critical section.

The source is now prevalidated in one-page scopes, including pixels clipped
outside scanout. Each visible row is copied through a 4 KiB kernel stack buffer.
A fresh mapping guard validates and copies at most 4 KiB (at most two pages for
an unaligned source), then ends before volatile device writes. No user pointer
or unlocked physical source view escapes a mapping scope. Source validation is
still required again at each copy; an Arc alone does not pin a mapped frame.

An ordinary display access lock spans the presentation and serializes all pixel
writers, lease changes and modesets. Metadata is held only around snapshots and
backend publication, and never while entering a user mapping scope. The order is
access → bounded VM mapping; metadata is acquired separately under access. IRQ
and NMI paths do not acquire display access; CPL0 timer interrupts do not dispatch
another application. Exception and panic diagnostics use serial. These facts do
not enable concurrent AP application execution or replace the remaining OP3 gates.

The ABI still validates the entire supplied source range before any pixel copy.
A concurrent unmap after preflight can now return EFAULT after earlier chunks
reached scanout. Presentation does not promise transactional pixel contents;
callers must retain their source mappings until the syscall returns. Device
ownership and geometry remain stable until copying and publication finish.

Host tests in `kernel/tests/terminal_display.rs` compile the actual driver and
syscall adapter. They cover clipping, unaligned multi-chunk rows, inaccessible
clipped source pages, per-chunk revocation, copied pixel values, ownership and
mode/release exclusion. `tools/test_user_memory.py` separately checks mapping
permissions and final-reader lifetime. These are host transport tests.

With boot-smoke-tests only, a one-shot >=1 MiB successful present reports
`[FB-PRESENT] mapping_max_bytes=… chunks=… irq_enabled_chunks=… pit_ticks=…`.
The interval starts after acquiring display access. PIT ticks come from the
actual delivered interrupt counter, never HPET-derived elapsed time. The marker
is published after display/mapping guards end and only when a PIT interrupt was
observed. It is absent from production. Native results are recorded separately;
byte-bounded copies do not establish a universal elapsed IRQ-off latency budget,
KVM scaling or physical input-to-photon timing.

The [18 November receipts](../verification/multitrack-next-2026-11-18.json)
record the full GUI budget on 2-vCPU TCG and a real 1280×720 presentation with
1440 chunks, all 1440 observing IF enabled after source-pin release, maximum
4096 copied bytes per pin and one delivered PIT interrupt during the presentation.
The old full-frame code and new native evidence establish shorter memory scopes;
these scene runs do not provide a comparable before/after FPS benchmark.
