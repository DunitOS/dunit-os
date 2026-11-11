# OP0 — first reproducible UP measurements

11 November 2026. [Data](op0-up-baseline.json) records a controlled TCG run:
1920×1080, 512 MiB, one requested/emulated vCPU, software rendering. KVM is not
available on this runner. This is an initial baseline, not a latency/scaling SLA.
The captured image hash and source fingerprints identify the pre-CPU-local tree.

```sh
python3 tools/performance_baseline.py --output build/performance/up-baseline \
  --accel tcg --cpus 1
```

The existing latency fixture and its entire correctness/DF budget are retained.
Trial 1 warms each UI scene; five normal trials are measured. The separate wheel
burst remains mandatory. Each trial verifies exact 24-detent/character results,
selection identity and undo state before reporting completion. Fixture compilation
explicitly uses the same size-oriented fat-LTO/z profile as production GUI clients.

| Scene | Median stream completion ms | Measured trials |
| --- | ---: | ---: |
| 24 wheel detents | 1019.36 | 5 |
| 24 short-field characters | 721.33 | 5 |
| 24 long-field characters | 1954.46 | 5 |
| Drag selection | 273.06 | 5 |

These host wall intervals include injection/polling; they are not per-event or
physical scanout latency. Empirical p95/p99 are preserved with raw values, clearly
labelled as insufficient to establish tail latency from five trials. Client
render/publication/queue counters accompany each sample.

The C fixture is compiled by native TCC. Four pthread cases share the exact same
4,194,304 work units and checked checksum 9,007,160,971,231,232. Each worker count
1/2/4/8 has one warmup and five timed runs; thread creation/join are included.
Allocator and MemFS I/O check all touched/read bytes and resource cleanup. Median
coarse guest elapsed times are 40/50/50/50 ms for pthread counts 1/2/4/8, 230 ms
for allocator and 30 ms for I/O. **All still execute on the UP scheduler.**
PIT's 10 ms granularity and IRQ-off undercount prevent fine-grained CPU-time claims.

Opt-in harness metadata records QEMU command/version, image hash, source status
and hashes, compiler versions, actual accelerator, host affinity/quota, requested
vCPUs and QMP's emulator count. Missing CPU time/RSS/scanout metrics remain null;
emulator CPU count is not substituted for kernel online/dispatch CPU count.

The full test fixture exposed the ESP budget. Unreachable kernel sections are
now collected at link time, retaining linker-script KEEP roots. The full-image
native baseline and DF/IRQ tests passed without increasing the 128 MiB ESP,
removing applications or weakening marker assertions. Subsequent NOLOAD/BSS
layout work is documented in the SMP receipt, not attributed to these timings.

OP0 remains open for the IPC microbenchmark, a stable KVM reference platform,
per-event high-resolution latency/accounting and the rest of the workload matrix.
Raw VM images/serial logs stay in build artifacts; structured measurements are
retained here, following the existing performance documentation convention.
