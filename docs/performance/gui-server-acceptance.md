# M7.9 acceptance — 4 November 2026

The retained shell uses the shared DSS theme. Extraction is not enabled; the
[architecture decision](../sdk/gui-server-decision.md) records the experiment,
protocol gaps, boot/recovery cost and why the external-shell gate is not met.
This completes the retained-shell branch of M7.9, not the ≤16 ms performance goal.

## Controlled workload

Baseline: `ef144aa`, immediately before the final metrics/geometry migration.
After: the same decoder, kernel, protocol, SDK input semantics, configurations,
font and unchanged latency contract. QEMU TCG, one vCPU, 512 MiB, software
1920×1080 framebuffer, a 640×360 client, animations/widgets/notifications off.
The kernel is UP; no KVM/SMP result is inferred. All OS runs use `qemu_test.py`
through the existing fixture wrapper. No input/marker assertion is relaxed.

Each row is the median **total host completion time for one stream**, including
injection and marker polling, over three trials. It is not physical per-event
input latency. The original atomic wheel burst is reported separately.

| Scene | Before ms | First after ms | Repeat after ms | Final ms |
| --- | ---: | ---: | ---: | ---: |
| 24 wheel detents | 545.3 | 582.3 | 519.8 | 703.1 |
| 24 characters, short field | 537.7 | 877.5 | 521.4 | 642.7 |
| 24 characters, 400-line field | 1400.8 | 1639.2 | 1312.3 | 1509.1 |
| Drag selection, 20 motions | 278.2 | 315.4 | 277.5 | 293.6 |
| Atomic 24-detent burst, one trial | 47.5 | 47.5 | 48.0 | 50.2 |

The whole-loop `frame ~N ms` marker is **21 → 21 → 21 → 21 ms**. This is the explicit
frame-marker acceptance gate. No stylesheet parse, token map lookup or resource
read was introduced into compositor painting. Source/configuration boundaries
prepare the typed values and assets.

The first after run was slower. A fresh repeat, with local host checks finished,
returned to baseline-level stream medians. The final run after adding shared
startup placement was slower again. All three after runs are retained in
[raw trial summaries](gui-server-results.json); neither is silently discarded.
The observations do not establish that host load caused all variance, a general
throughput improvement, or statistical absence of every input regression.
They do establish identical completed state assertions and an unchanged frame
marker. This record does not accept external-shell performance on that basis.

Client aggregate render/transport timings and queue maxima are included in the
JSON. The guest PIT clock has 10 ms resolution and can undercount IRQ-masked
work; stages include scheduling and do not form an exclusive CPU profile.
Aggregate server windows contain different mixtures of setup/input/idle frames
and are not paired per-scene CPU benchmarks. No unexposed kernel pointer queue
depth or physical scanout latency is invented.

## External SDK publication/isolation experiment

A separate 640×28 themed panel-like client receives 24 space key makes with the
same 5 ms injection spacing used by the established latency fixture. Measurements
start after explicit arming, excluding initial configure/resizes. All 24 model
updates must result in 24 commits and 24 actual FrameDone notifications.

| Observation | Measured value |
| --- | ---: |
| Command submitted → compositor and panel ready | 1399.6 ms |
| 24 input-driven panel publications, host total | 502.0 ms |
| Guest redraw total across those publications | 470 ms |
| Guest FrameDone wait total | 470 ms |
| Client input queue high-water | 35 packets |
| Intentional panel fault → survivor finishes and closes, host total | 525.6 ms |
| Survivor publications after peer loss | 24 |
| Whole-loop frame marker in the probe scene | 20 ms |

Boot timing starts after typing `exec /app/gui_server`, not at power-on. Recovery
time includes waiting for the compositor's exact reap marker, explicitly arming
the survivor through QMP, and 24 new publications; it is not just the
compositor's fault-detection latency. Zero guest render time means
below clock resolution, not zero CPU cost. Queue depth includes key-up/lifecycle
packets. The normal retained shell does not pay a client IPC/FrameDone round-trip
for each internal control change; this probe adds a real separately scheduled
publication path, not a complete shell replacement comparison.

The exact panel process faults with `invalid-opcode`, is reaped with
`kind=3 code=-4`, and only then does the harness arm the survivor for its 24
publications. The existing compositor serves it without a restart. The verifier rejects any other post-ready client fault, missing frame
completion or clock failure. The initial unpaced exploratory keyboard burst
could overrun the existing input queue; it is not used as a fast throughput
sample. The final generator uses the established 5 ms spacing and still requires
all 24 changes. This does not claim arbitrary keyboard bursts are lossless.

[Probe counters and host timings](gui-server-shell-probe.json) are retained.
The existing protocol still lacks real panel roles/layers/reservations and
external DWM-policy operations. Isolation succeeds for ordinary clients;
full-shell equivalence, performance and bootstrap/recovery acceptance do not.
The shell therefore stays inside `gui_server`, exactly as allowed by the roadmap.

## Validation

- 175 existing DWM/geometry/input/config/effect/editor host checks passed.
- 11 shared SDK/compositor theme tests passed, including custom/clipped control
  geometry, explicit inheritance and invalid candidates.
- 10 appearance-source mutation checks and 7 strict probe-report checks passed.
- Native latency: baseline plus three after runs, each 46 required/6 forbidden.
- Native theme: 10 required/6 forbidden; three unchanged client goldens plus
  exact panel/cursor/control pixels. The light profile moves its close chip
  through DSS; invalid profiles retain the prior scene.
- Native external-process probe: 12 required/5 forbidden plus exact PID/fault,
  24-publication/completion and survivor assertions.

No ABI or wire-format change; no marker/test removal; generated build products
and screenshots are not committed. GUI smoke passed its unchanged 30 required/13 forbidden contract. Its loaded
frame marker was 112 ms with `reaped_to=2 reopened=6`; the earlier scalar-save
checkpoint had 98 ms with `reaped_to=4 reopened=4`. These variable recycled
client scenes are not the matched timing workload above; no loaded-scene
performance improvement is claimed.
