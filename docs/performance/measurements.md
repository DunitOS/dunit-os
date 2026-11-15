# Performance measurements

The system currently dispatches applications on one CPU. These records establish
correctness and bounded workload observations; they do not accept OP4 scaling,
physical input latency or the remaining OP0/OP5–OP9 goals. Current ownership and
open safety gates are in [SMP foundations](smp.md). GUI experiments are in
[GUI performance](gui-performance.md).

## Measurement contract

Every comparison must preserve work counts, checksums/state assertions, marker
budgets, source/image identity, toolchain/profile, configurations and host setup.
Metadata distinguishes requested/emulated CPUs from online/kernel dispatch CPUs,
actual accelerator, clocksource resolution from event source, QEMU command/version,
host affinity/quota and source status/hashes. Unmeasured CPU time/RSS/scanout stays
null; emulator CPU count is never application concurrency evidence.

GUI stream completion uses host wall time from QMP injection to the final serial
completion marker, including approximately 20 ms polling resolution. It is not
per-event or physical scanout latency. Five samples cannot establish p95/p99 tail
guarantees. Aggregate client/server stages include UP scheduling; different
profiler windows are not paired exclusive CPU measurements.

Historical PIT clock values have 10 ms resolution and undercount IRQ-masked
framebuffer/serial work. Zero means below resolution. One-tick shifts, shared
TCG host noise and environment restarts do not establish speedup. Old whole-loop
`frame ~N ms` includes pacing/sleep and is not compositor CPU cost. Host wall time
is the primary comparison for those historical GUI scenes.

The current clocksource validates ACPI RSDP/RSDT/XSDT/HPET checksums, table bounds,
memory GAS/alignment, 64-bit width, period and counter progress. Firmware aliases
retain cache attributes and are retired; the permanent HPET device mapping is
published before AP startup. Missing/invalid/32-bit/stuck HPET falls back to PIT.
There is no guessed device address or unvalidated cross-CPU TSC arithmetic.

Wide saturating HPET conversion supplies monotonic nanoseconds even with IRQs
masked; deadline conversion rounds upward into existing scheduling ticks. QEMU
HPET's advertised 10 ns period is counter resolution, not accuracy/wake latency.
PIT remains the 100 Hz event source. Native checks prove at least 2 ms IRQ-off
progress with unchanged PIT delivery count, and bracket actual AP IPI counter
reads with BSP reads. `-machine hpet=off` proves the 10 ms PIT fallback; this QEMU
rejected an earlier `-no-hpet` command before boot. Clock changes invalidate
direct guest-time comparisons with older PIT series.

## UP baseline

The 11 November [OP0 baseline](op0-up-baseline.json) uses one requested/emulated
CPU, QEMU TCG, 512 MiB, software 1920×1080 display and a 640×360 GUI fixture.
No usable KVM was available. One warmup plus five measured trials retain exact
24-detent/character results, selection/undo assertions and the separate wheel
burst. Fixture compilation uses production size-oriented fat-LTO/z settings.

| GUI stream | Median host completion ms |
| --- | ---: |
| 24 wheel detents | 1019.36 |
| 24 short-field characters | 721.33 |
| 24 long-field characters | 1954.46 |
| Drag selection | 273.06 |

Native TCC CPU scenes use exactly 4,194,304 work units and checksum
9,007,160,971,231,232 with 1/2/4/8 pthread workers, including creation/join.
One warmup and five trials per scene gave coarse PIT medians 40/50/50/50 ms;
allocator and MemFS I/O medians were 230/30 ms with touched/read bytes and final
resource cleanup verified. Every worker still executes on the UP scheduler.

The baseline exposed the 128 MiB ESP budget. Section GC keeps linker-script roots;
later explicit GOT placement before NOLOAD BSS prevents a file-backed zero-fill
hole. The ELF check rejects PROGBITS BSS or oversized file-backed LOAD segments.
The earlier kernel was 6,129,736 bytes after GC; the bring-up build was 3,603,768
bytes, with 2,539,672 BSS bytes and a 455,336-file/2,998,424-memory-byte data LOAD.
These are different builds and footprint observations, not runtime speedups.
No applications were removed or ESP enlarged. [Bring-up data](smp-bringup.json)
records the later layout checks.

## IPC baseline

The subsequent [wait/clock/IPC receipt](progress-foundation.json) adds the OP0 IPC
microbenchmark missing from the initial baseline. `assets/tcc/performance_ipc.c`
is compiled by TCC inside Dunit; two separately spawned processes exchange
authenticated packets. One warmup plus five measured trials each verify 1,000
round trips, 64,000 bytes, sender identity, sequence/payload/checksum and child
exit. Checksums are validated per trial; the report rejects incomplete work.

Median elapsed time per round trip was 68,616.11 ns on the smoke image and
67,533.45 ns on production, calculated from complete fixed-work trials with HPET.
These are UP round-trip observations, not per-packet tails, parallel scheduling
or input latency. Each JSON record is emitted in one write. A stable KVM platform,
per-event latency/accounting and the remaining workload matrix are still open.

## Historical comparisons

Three ownership changes retain coarse before/after fixed-work series. Each uses
one vCPU, TCG, 512 MiB, one warmup plus five measured trials, identical work and
checksums in all 36 records. The before image hash is checked against its earlier
committed receipt rather than inferred from an already edited worktree.

| Scene | Queue before/after ms | Dispatch before/after ms | Registry before/after ms |
| --- | ---: | ---: | ---: |
| CPU, 1 worker | 50 / 50 | 50 / 40 | 50 / 50 |
| CPU, 2 workers | 50 / 50 | 60 / 50 | 40 / 40 |
| CPU, 4 workers | 60 / 50 | 50 / 50 | 50 / 50 |
| CPU, 8 workers | 50 / 50 | 60 / 50 | 50 / 50 |
| Allocator | 240 / 230 | 280 / 240 | 260 / 230 |
| MemFS I/O | 30 / 30 | 30 / 30 | 30 / 30 |

[Queue records](scheduler-queue.json), [dispatch records](dispatch-ownership.json)
and [registry records](process-registry.json) preserve provenance and trials.
Registry runs span a host restart. PIT quantization/IRQ-off undercount and TCG
noise prevent a speedup or scheduling-tail conclusion. Whole test invocation
times in ownership receipts are correctness run durations, not syscall latency.

Failed setup/test attempts stay in their receipts. The latest PTY boot initially
used an outdated PIT-only clock budget; the actual q35 HPET/PIT-event selection
was then asserted with every other boot requirement retained. The earlier task
scope signal fixture initially assumed blocking wait; it was corrected to yield
and retry EAGAIN while preserving signal/exit assertions. Neither failure is
turned into a passing performance sample.

## Reproduction

OS builds and native tests use the existing tools/qemu_test.py harness through
these focused wrappers. Serialize builds/VMs sharing build artifacts and QMP.
Use each receipt's source/image/configuration for a historical reproduction;
running current sources produces a new series.

```sh
python3 tools/performance_baseline.py --output build/performance/up-baseline --accel tcg --cpus 1
python3 tools/test_smp_boot.py
python3 tools/test_pmm_smp.py --cpus 1 4 8 --accel tcg
python3 tools/test_process_resources.py --cpus 4 --accel tcg
python3 tools/test_process_registry.py --reuse-image --cpus 1 4 8 --accel tcg
python3 tools/test_execution_native.py --reuse-image --cpus 1 2 4 8 --accel tcg
python3 tools/test_clock_native.py --reuse-image --cpus 1 4 8 --accel tcg
python3 tools/test_tlb_native.py --reuse-image --cpus 1 4 8 --accel tcg
python3 tools/ipc_performance_report.py build/qemu-serial.full.log
```

Host contracts cover actual guards, process access, execution ownership, scheduler
and wake queues, parking, PMM, futex/UserWord, IPC, clock, TLB, user memory, PTY and
QMP transport using their tools/test_*.py entry points. Native production marker
budgets separately forbid boot-only proof callbacks. DunitFS persistence checks
use prepare/verify boots on the same disposable image, preserving disk contents.

A terminal-image GUI adapter retains the complete original actions and marker
budgets, attaching actions to explicit compositor launch:

```python
import json
from pathlib import Path
m = json.loads(Path("tools/gui_smoke_markers.json").read_text())
m["run"]["command_actions"] = {"exec /app/gui_server": m["run"].pop("actions")}
m["run"]["config"] = "limine_test_terminal.conf"
m["run"]["cmd"] = ["exec /app/gui_server"]
Path("build/performance/gui-markers.json").parent.mkdir(parents=True, exist_ok=True)
Path("build/performance/gui-markers.json").write_text(json.dumps(m))
```

Pass that manifest to qemu_test.py. The same harness supports QMP Unix sockets or
`--qmp-transport stdio` nonblocking pipes with bounded partial writes, fragmented
replies and event handling; stderr is retained per workdir. The pipe transport
supports hosts that disallow socket creation without introducing another runner.
Raw images/logs/screenshots stay in build artifacts; committed JSON receipts
remain the durable measurement record.
