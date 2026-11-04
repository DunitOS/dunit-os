# Continuous input: measured pipeline fixes (2026-11-02)

The uneven response was reproducible. Discrete controls require one transaction;
continuous input accumulates transactions faster than the old pipeline finishes
them. The main causes were repeated frame-boundary waits, expensive text work,
and unnecessary full-screen composition. Single-vCPU TCG amplifies all three.

## Controlled native measurements

All runs use the same 640×360 fixture on a 1920×1080 desktop, QEMU TCG, 512 MiB,
one vCPU (no `-smp`), software rendering, animations/widgets/notifications disabled.
This host has no `/dev/kvm`. The baseline already includes negotiated input
lifecycle and the same opt-in metrics; it is not an unmodified build of 67885f1.

Numbers are median **total completion time for a stream**, not latency per event
or physical input-to-scanout. Each scene runs three times. Timing starts before
QMP injection and ends at a serial marker after the last redraw/FrameDone;
marker polling adds approximately 20 ms resolution. All raw trials and client
stage totals are in [continuous-input-results.json](continuous-input-results.json).

| Scene | Baseline ms | Trace gated | Transport/pacing/drain | Final ms | Speedup |
| --- | ---: | ---: | ---: | ---: | ---: |
| 24 wheel steps | 4068.2 | 3580.6 | 2531.7 | 537.7 | 7.6× |
| 24 characters, short field | 2046.6 | 1996.4 | 544.4 | 519.9 | 3.9× |
| 24 characters, 400-line field | 5768.8 | 5373.2 | 4863.8 | 1372.7 | 4.2× |
| Drag selection, 20 motions | 661.6 | 617.7 | 454.3 | 253.1 | 2.6× |

The historical fixture asserted exactly 24 wheel messages and the exact resulting offset;
current checks count detent units and also cover aggregated packets (see below);
typing asserts all 24 characters and their value. Drag asserts an unchanged
value, a nontrivial selection, and unchanged undo state. Requests, wheel records,
key makes and button edges are not silently dropped or merged to obtain a gain.
Explicit QMP key down/up chords avoid a test-generator trap: `send-key`'s delayed
virtual-time Ctrl release originally turned `a` into Ctrl+A under TCG. Those
failed setup runs are excluded, rather than counted as fast typing.

## Ranked causes, evidence and changes

1. **A publication transaction crossed several compositor iterations.**
   `runtime/dunit/src/window.rs` previously waited for Import, Attach, Commit,
   FrameDone and retirement separately; the server only pumped requests on its
   frame iteration. Baseline short-field totals were 770 ms in Import/Attach,
   480 ms in Commit result and 510 ms in retirement across 24 redraws.
   `frame_runtime.rs` now serves IPC through wait-set until an absolute frame
   deadline; Window sends CAP/Import/Attach/Commit in FIFO order before waiting
   for all results and the matching actual FrameDone. The transport-stage run
   reduced short-field time to 544 ms. BufferRelease plus DestroyResult remains
   the last-reader barrier; callbacks do not complete before composition.

2. **Repeated text layout and glyph rasterization dominated long input.**
   `dunit-render/text_input.rs` and component renderer formerly rebuilt a full
   VisualLines map at least five times per render, including offscreen text.
   PreparedInput now shares one immutable map within a render and subsequent
   hit-testing. Renderer owns a glyph coverage cache, bounded by both 512 entries
   and 1 MiB payload, with exact size/font identity, negative entries and LRU
   eviction. Font remains Send+Sync. A quiet host release benchmark isolates
   render work from IPC, scheduling and QEMU (24 events, warmup plus five trials):

   | Scene | Baseline render ms | Shared layout | Plus glyph cache |
   | --- | ---: | ---: | ---: |
   | Wheel | 36.506 | 35.790 | 10.180 |
   | Short field | 9.264 | 8.663 | 7.682 |
   | Long field | 131.853 | 87.013 | 46.111 |

   Damaged frame/pixel counts are identical in every stage. Cached/uncached
   rasterization and prepared/unprepared text rendering have exact pixel tests.
   Five samples per scene are in [input-render-results.json](input-render-results.json).

3. **The stationary software cursor blocked partial damage.**
   `damage_runtime.rs` treated its rectangle as an occluder: scrolling or typing
   beneath it forced a full screen even with `damage_blocked=false`.
   The stable partial path now copies client damage, redraws the same opaque
   cursor, and presents the damaged rectangles. Moving cursors and other unsafe
   scene changes retain the full-frame fallback. A pixel test compares partial
   updates with fresh full composition, including clipped cursor positions.
   Native final profiler windows for stationary typing report `full=0`, with
   69/39/41 bounded presents rather than full-screen presents; the baseline
   presents in those windows were full-screen. Window boundaries differ between
   runs, so aggregate 128-frame counters are not a paired per-scene benchmark.

4. **One pointer record per frame and a sleep after work added backlog.**
   The kernel queue has 64 records, coalesces only pure motion, and marks overflow
   LOST_BEFORE. The compositor now drains at most 64 records per iteration,
   preserving order, wheel direction, edges and loss handling. Frame pacing uses
   start plus configured interval (default 16 ms), rather than work plus sleep.
   Queue pressure can move to the client: wheel input high-water was 20 before
   and 23 after; short typing fell from 45 to 39. The IPC queue maximum sampled
   by the server fell to 8 in the final run. Kernel pointer occupancy is not
   exposed by the current ABI, so no occupancy number is claimed. One final
   profiler loss flag includes the setup/positioning period; exact measured
   wheel counts and text values still pass. Overflow behavior remains enabled.

5. **Successful IPC calls unconditionally wrote serial trace lines.**
   Only the three success-path send/receive traces now use the existing debug
   trace switch. Errors and test markers remain. The isolated trace-only run
   improves wheel by 12%, long typing by 7%, short typing by 2%: material under
   TCG, but insufficient to explain the original lag on its own.

## Timing limits and remaining work

The old `gui_server: frame ~N ms` measures the whole iteration including sleep;
it is not compositor CPU cost. Earlier local idle logs were around 20–24 ms;
the reported 29–35 ms idle render cost was not independently reproduced as such.
PIT runs at 100 Hz and ClockGetTime advances in 10 ms ticks; sleep(16) rounds to
ticks. Syscall entry masks interrupts, so long framebuffer/serial critical
sections can undercount guest elapsed time. Client render/stage measurements
include UP scheduling, are quantized, and can report zero below a tick. Server
work and idle-pump stages cover different intervals and must not be added as if
they were exclusive CPU profiles. Host wall time is the primary comparison.

The system remains UP and software rendered. No claim about KVM speedup or SMP
is made. Clients still redraw synchronously for each handled event; this patch
reduces the cost without changing delivery semantics. Long-field editing still
rebuilds its full value map once per render. A versioned kernel pointer modifier
snapshot was a separate pending ABI decision at measurement time; it was
subsequently [approved and implemented](../sdk/pointer-modifier-snapshot-proposal.md)
in the M7.6 widget completion increment. These recorded timings predate it.

## Reproduction and validation

Run OS tests only through the standard wrapper, which invokes qemu_test.py:

```sh
python3 tools/test_sdk_gui_fixture.py latency --accel tcg
python3 tools/sdk_latency_report.py --run final timing.log build/qemu-serial.full.log
cargo run --release --manifest-path runtime/dunit-component/Cargo.toml --example input_render_bench
```

Enable `[diagnostics] performance = true` for aggregate server metrics (every
128 frames), and `ComponentWindow::enable_metrics(true)` for client counters.
Both are off by default. No per-event profiling logs are added. The latency
fixture and lifecycle/legacy fixture require actual state assertions, successful
client exits, boot markers and no GUI faults. Resize and theme regressions cover
concurrent configure/recovery and exact themed pixels respectively.

## IntelliMouse ID 3 correction — 4 November 2026

A later native latency run exposed an additional kernel-side problem. The
mouse driver negotiates device ID 3 (200/100/80 sample-rate sequence), whose
fourth packet byte is a signed **eight-bit** delta. It was applying the signed
four-bit Explorer ID 4 interpretation. A positive packet of 18 detents became 2;
24 became -8. This loses scroll distance or reverses it even before the kernel
pointer queue. The driver now preserves the complete signed byte.

Two production-decoder host tests cover both directions across the nibble
boundary and equivalent aggregated/individual detent totals. Both fail when the
old decoder is substituted. QEMU's original three wheel rounds are unchanged.
An additional `wheel-4` regression injects 24 down detents in one QMP sync and
asserts an actual packet >=8, total exactly 24 and the exact scroll displacement:

```text
[PERF-WHEEL] wheel-2 delta=18 accepted=22
[PERF-WHEEL] wheel-4 delta=24 accepted=24
[PERF] wheel-4 done
```

The burst completed with one render/commit and a client input queue high-water
mark of 1. Its host completion time was 67.7 ms in this run, including injection
and marker polling. This is a separate regression workload, not a speedup over
24 individually delivered events. `sdk_latency_report.py` keeps it separate
from the original three-trial wheel median and still reads historical twelve-
phase logs. The original model checks remain; the native budget now has 46
required markers and 6 forbidden markers, all passing. This correction explains
corruption for large wheel packets; it does not establish the cause of every
previous intermittent stall or imply that the synchronous render loop is fixed.
