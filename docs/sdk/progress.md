# Progress and busy indicators (M7.6)

```rust
View::progress("download", "Download", self.received,
    ProgressOptions::new(0, self.total))
View::busy("working", "Working", self.running)
```

These are passive, model-owned indicators. They never focus, capture the pointer,
activate or dispatch messages. Tab skips them; application buttons, shortcuts or
background work update the model normally. Use a separate Text node for a visible
label or percentage. `accessibility_role()` returns `progressbar`,
`accessibility_label()` returns the supplied name, and `progress_state()` exposes
either `Determinate { value, options }` or `Busy { running }`.
`accessibility_busy()` is true only for a running busy indicator; busy has no
invented numeric percentage.

## Values and appearance

`ProgressOptions { min, max }` accepts an inclusive `i64` range and values within
it. Invalid/reversed/out-of-range values fail view validation with
`UiError::InvalidProgress`. Intermediate subtraction uses `i128`, including the
full signed integer range. A zero-span range is complete. Determinate fill grows
from left to right, with no speculative interpolation or callbacks.

A running busy indicator shows a bounded segment moving left-to-right and back.
Paused busy shows the empty track. Disabled indicators retain their model state
and use the theme's muted fill; a disabled busy indicator shows a stationary
centered segment. Reduced motion and immediate untimed rendering also show a
stationary centered segment for running busy indicators.

Dark/light profiles provide all appearance and timing through DSS:
`progress-thickness`, `progress-min-track`, `progress-radius`,
`progress-track-color`, `progress-fill-color`, `busy-segment` (0..1),
`busy-duration` and `busy-interval` (milliseconds). Typed SDK theme validation
rejects wrong types, negative dimensions/times and out-of-range segment sizes.
The generic unthemed Progress measurer keeps its previous defaults.

## Sampling and retained rendering

The existing window wait-set samples busy indicators; no extra thread, syscall
or gui-v1 message is required. The default profile uses a 1200 ms round trip and
40 ms sampling interval. Multiple visible busy indicators share the earliest
requested interval with paint-property transitions. Their phase uses the
window's monotonic clock, so unrelated model updates and resize do not restart
an operation's displayed cycle. This is a visual cycle, not elapsed task time.

`Renderer::render_at` accepts an explicit clock for deterministic host frames.
`render` uses the stationary representation. The renderer exposes
`busy_interval_ms()` and `RenderStats::active_busy_indicators`; the latter counts
visible moving indicators that currently require a timer.
`ComponentWindow::is_animating()` includes these indicators.

Paused, disabled, reduced-motion, zero-duration/interval, zero/full-segment and
fully clipped indicators require no busy timer. Removing the last busy node also
stops scheduling. Visibility includes the surface, ancestor rectangles and
scroll viewports; it does not infer whether another desktop window covers the
client. `cancel_motion` cancels pending sampling; a later timed render may start
it again. Stylesheet reload rebuilds the snapshot and applies the new cadence.

Progress and busy paint snapshots participate in retained damage. Geometry
includes border/padding and the theme's track thickness; clipping, scroll
translation, old-fill clearing and resize use the common renderer. Unchanged
values/phase skip publication. Dirty pixel counts describe invalidation area,
not a measured whole-desktop frame-time budget.

## Verification

Seven host scenarios cover range validation and integer extremes, accessibility,
passive routing/focus, exact geometry, dark/light pixels, damage and resize,
busy phase/cycle, reduced motion, pause/disable, timing edge cases, scroll
clipping, removal, multiple cadences and theme validation/reload. Existing SDK
host suites run as regressions.

```sh
python3 tools/test_sdk_gui_fixture.py progress --accel tcg
```

One native scenario checks real timed pixel changes while waiting for events,
reduced-motion and paused states with no timer redraws, keyboard model updates,
Tab skipping passive fields, a real passive mouse click, completion,
resize/focus and resuming animation. It runs through `tools/qemu_test.py` with
`tools/sdk_progress_markers.json` and captures `build/sdk-progress.ppm`.
