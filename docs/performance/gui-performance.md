# GUI performance and shell acceptance

These experiments use the UP scheduler and software rendering. Stream timings
measure host completion, not per-event physical input-to-scanout. Current kernel
foundations and measurement limits are described in [smp.md](smp.md) and
[measurements.md](measurements.md#measurement-contract).

## Continuous input

The 2 November experiment reproduced uneven wheel/typing/selection response.
Repeated publication waits, text work and full-screen composition accumulated
backlog under one-vCPU TCG. Controlled scenes use a 640×360 client on a
1920×1080 desktop, 512 MiB, animations/widgets/notifications off. The baseline
already includes negotiated input lifecycle and opt-in metrics; it is not an
unmodified 67885f1 image. Each scene has three trials.

| Stream | Baseline ms | Trace gated ms | Transport/pacing/drain ms | Final ms |
| --- | ---: | ---: | ---: | ---: |
| 24 wheel steps | 4068.2 | 3580.6 | 2531.7 | 537.7 |
| 24 short-field characters | 2046.6 | 1996.4 | 544.4 | 519.9 |
| 24 characters, 400-line field | 5768.8 | 5373.2 | 4863.8 | 1372.7 |
| Drag selection, 20 motions | 661.6 | 617.7 | 454.3 | 253.1 |

[All trials and stage counters](continuous-input-results.json) retain exact wheel
offset/detent totals, all 24 text characters and selection identity without value
or undo changes. Requests, wheel units, key makes and button edges are not
dropped to obtain faster completion. Explicit QMP down/up chords avoid delayed
Ctrl release accidentally generating Ctrl+A under TCG; failed setup trials are
excluded from timing, not counted as fast typing.

Window publication pipelines CAP/Import/Attach/Commit in FIFO order before
waiting for their results and matching actual FrameDone. Server wait-set pumping
serves requests until an absolute frame deadline. Earlier short-field stages
spent 770 ms in Import/Attach, 480 ms in Commit result and 510 ms in retirement
across 24 redraws. The transport-stage stream fell to 544 ms. BufferRelease plus
DestroyResult remains the final-reader barrier; callbacks do not finish before
composition.

Pointer draining is bounded at 64 records per iteration, preserving order,
wheel direction, edges and LOST_BEFORE handling. Kernel capacity remains 64;
only pure motion coalesces. Pacing uses frame start plus configured interval
(default 16 ms), rather than work plus sleep. Queue pressure can move to clients:
wheel high-water 20 -> 23, short typing 45 -> 39, sampled final IPC maximum 8.
Kernel pointer occupancy is unexposed and not claimed. A setup-period loss flag
does not waive exact measured input/state checks or disable overflow handling.

The three successful IPC send/receive traces use the existing debug switch;
errors/test markers remain. Trace gating alone improves these streams by about
12% wheel, 7% long typing and 2% short typing, insufficient to explain all lag.
No per-event profiling logs were added. Optional server metrics report every
128 frames; client metrics are opt-in. Both are off by default.

### Idle commit wakeup

Once the allotted interval from the last presenting frame has elapsed, a
commit for an already visible window can end an idle desktop wait. The
regular cadence still applies after a presentation. FrameDone still follows
composition and does not change the buffer release barrier.

In one-vCPU TCG, the long-field fixture's median completion time over six
trials fell from 1365.1 ms to 854.4 ms. Its cumulative FrameDone wait across
24 characters fell from 573.5 ms to 3.6 ms. The other scenes have overlapping
before and after ranges. [Raw trials and wake counts](idle-commit-wake-2026-11-21.json)

## Render cost

PreparedInput shares one immutable VisualLines map within rendering/hit-testing,
replacing repeated full/offscreen layout. Renderer caches glyph coverage by exact
font/size identity, including negative entries, with LRU limits of 512 entries
and 1 MiB payload. Font remains Send+Sync. Long-field editing still rebuilds its
full value map once per render; clients still redraw synchronously per event.

The quiet-host release benchmark isolates render work from IPC/QEMU/scheduling:
24 events, warmup and five measured trials. [Raw render samples](input-render-results.json)
retain identical damaged frame/pixel counts in every stage.

| Scene | Baseline render ms | Shared layout ms | Plus glyph cache ms |
| --- | ---: | ---: | ---: |
| Wheel | 36.506 | 35.790 | 10.180 |
| Short field | 9.264 | 8.663 | 7.682 |
| Long field | 131.853 | 87.013 | 46.111 |

Cached/uncached and prepared/unprepared rendering have exact pixel checks.
The 21 November SDK painter change clips each glyph once and writes visible
rows directly. It also bulk-fills opaque rectangles. In the five-trial host
benchmark, median wheel render time fell from 9.65 ms to 4.22 ms for the same
24 events. Under one-vCPU TCG, wheel render time fell from 117.7 ms to 57.8 ms.
Wheel stream completion fell from 595.5 ms to 545.6 ms, with overlapping trial
ranges. The typing and selection streams showed mixed completion times.
[Matched trials](surface-blit-2026-11-21.json)

Stationary opaque cursors no longer block partial damage: copy client rectangles,
redraw the cursor, present bounded damage. Moving cursors/unsafe scene changes
retain full composition. Clipped-cursor tests compare with fresh full pixels.
Final stationary-typing windows report `full=0` and 69/39/41 bounded presents;
their 128-frame windows differ and are not paired scene CPU benchmarks.

## Pointer decoder

The 4 November native run exposed ID 3 IntelliMouse decoding as signed four-bit
Explorer ID 4. ID 3's fourth byte is signed eight-bit: +18 became +2 and +24 became
-8 before queuing. The production decoder now preserves the complete byte.
Host tests cover both signs beyond the nibble boundary and equal aggregated/
individual detent totals, failing against the old decoder.

The original wheel rounds remain. A separate wheel-4 injects 24 detents in one
QMP sync and requires a packet >=8, exact total 24 and exact displacement:

```text
[PERF-WHEEL] wheel-2 delta=18 accepted=22
[PERF-WHEEL] wheel-4 delta=24 accepted=24
[PERF] wheel-4 done
```

One burst run completed in 67.7 ms with one render/commit and client high-water 1.
It is a different workload from 24 separate events. The reporter keeps it
separate and supports historical twelve-phase logs. The native contract retains
46 required/6 forbidden markers. Decoder correction does not establish every
intermittent stall's cause or complete synchronous-loop optimization.

The pointer modifier snapshot was pending at the original measurement, then
[approved and implemented](../sdk/input.md#pointer-modifier-snapshots) during
M7.6; those timings predate that ABI increment.

## Retained shell

M7.9 keeps shell/DWM policy inside gui_server with shared DSS appearance, as
recorded in the [architecture decision](../sdk/desktop-transfers.md#retained-shell-decision). The
retained-shell branch is accepted; the <=16 ms performance goal remains open.

The 4 November matched workload uses baseline ef144aa before final metrics/
geometry migration, the same decoder/kernel/protocol/configuration/font/input
semantics and unchanged latency assertions. TCG/UP/display/fixture settings match
the continuous-input setup. Each stream has three trials; the burst has one.

| Stream | Before ms | First after ms | Repeat after ms | Final ms |
| --- | ---: | ---: | ---: | ---: |
| 24 wheel detents | 545.3 | 582.3 | 519.8 | 703.1 |
| 24 short-field characters | 537.7 | 877.5 | 521.4 | 642.7 |
| 24 long-field characters | 1400.8 | 1639.2 | 1312.3 | 1509.1 |
| Drag selection | 278.2 | 315.4 | 277.5 | 293.6 |
| Atomic 24-detent burst | 47.5 | 47.5 | 48.0 | 50.2 |

The whole-loop frame marker stayed 21 ms in every run. Painting does not parse
stylesheets, look up token maps or read resources; source/configuration boundaries
prepare typed values/assets. [All after runs](gui-server-results.json) are kept,
including the slower first/final runs. The repeat returned near baseline after
local host checks finished, without proving host load explains all variance,
absence of every regression or general throughput improvement. Exact final
state and the frame-marker gate passed.

[Earlier shell-style results](shell-style-results.json) retain the separate
appearance checkpoint. Loaded/recycled GUI smoke showed 112 ms with
`reaped_to=2 reopened=6`, versus an earlier 98 ms with `reaped_to=4 reopened=4`.
These different mixed scenes are not the matched experiment or a speedup proof.

## Shell isolation

The separate 640×28 themed panel-like SDK client receives 24 space key makes at
5 ms spacing after explicit arming/configure. Every update must produce one
commit and actual FrameDone. [Probe receipts](gui-server-shell-probe.json) record:

| Observation | Value |
| --- | ---: |
| Command submission to compositor/panel ready | 1399.6 ms |
| 24 publications, host total | 502.0 ms |
| Guest redraw / FrameDone wait totals | 470 / 470 ms |
| Client queue high-water | 35 packets |
| Intentional peer fault through survivor close | 525.6 ms |
| Survivor publications after loss | 24 |
| Whole-loop frame marker | 20 ms |

Startup begins at `exec /app/gui_server`, not power-on. Recovery includes exact
reap detection, QMP survivor arming and 24 new publications, not merely fault
detection. Zero measured render cost is below PIT resolution. Queue depth includes
key-up/lifecycle packets. The existing internal shell does not pay this separate
client IPC/FrameDone transaction for each internal control change.

The exact panel PID faults with invalid-opcode, is reaped with `kind=3 code=-4`,
then the existing compositor completes the survivor without restart. Verification
rejects other faults, missing frames or clock failure. An initial unpaced burst
could overrun the queue; only the established 5 ms generator is measured, still
requiring all 24 updates. Arbitrary keyboard bursts are not claimed lossless.

Ordinary client isolation passed. Real panel roles/layers/reservations and external
DWM-policy operations are missing from the protocol. Full-shell equivalence,
bootstrap/recovery and performance gates remain unmet; extraction is not enabled.

## Validation

Existing host checks covered 175 DWM/geometry/input/config/effect/editor cases,
11 shared-theme cases, 10 appearance-source mutations and 7 strict probe-report
checks. Native latency retained 46 required/6 forbidden markers for baseline and
three after runs; native theme retained 10/6 including exact client/panel/cursor
pixels and invalid-profile rollback. The isolation probe retained 12/5 plus exact
fault/PID/24-publication/survivor assertions. GUI smoke retained 30/13.

Resize/lifecycle tests keep actual state, exits, frame completion and no-fault
budgets. No marker assertions, delivery semantics or buffer retirement barriers
were removed. Generated products/screenshots stay outside committed sources.

```sh
python3 tools/test_sdk_gui_fixture.py latency --accel tcg
python3 tools/sdk_latency_report.py --run final timing.log build/qemu-serial.full.log
cargo run --release --manifest-path runtime/dunit-component/Cargo.toml --example input_render_bench
```

The fixture wrapper uses tools/qemu_test.py. These links describe existing
evidence; this documentation consolidation does not produce new timing trials.
