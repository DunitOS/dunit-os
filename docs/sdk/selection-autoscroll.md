# Selection edge autoscroll

The SDK extends a captured text selection while the pointer stays outside the
visible text viewport. Input scrolls horizontally; Textarea scrolls on both
axes. The pointer may remain stationary: the native wait-set deadline advances
the viewport and selection without synthesizing mouse events.

`selection-scroll-interval` is a DSS duration and `selection-scroll-step` is
content pixels per tick. Shared dark/light profiles select 32ms and 12px. Zero
interval or step disables autoscroll. Durations must be in 0..60000ms and steps
in 0..1024px. A delayed frame advances one step without a catch-up burst.

Only a focused, enabled field with an active text-selection capture schedules
ticks. Read-only text remains selectable. Each axis stops at its content limit;
returning inside the viewport stops its movement. Release, window Leave, focus
change, field removal/disable and scrollbar gestures end selection capture.
A pending input event is delivered before the next timer step. No timer remains
active when neither axis can move.

Captured pointer positions clamp to the visible viewport, including ancestor
scroll clips. A tick uses the new scroll offsets to find the next UTF-8 scalar
position and extends the original selection anchor. It preserves the model
text and undo/redo history. The pending viewport is applied after caret fitting,
so scrolling across short LF lines does not reset the other axis. Published
paint snapshots remain immutable until the next successful render. Captured
pointer placement pins both offsets as well, so motion between timer ticks
also preserves the other axis. Interval queries use cached axis measurements
without copying text or measuring glyphs.

Pure adapters use `Renderer::input_drag_index_at`, `input_autoscroll_ms` and
`input_autoscroll_step`, then `Application::drag_selection`. The application
method accepts only the currently captured, focused enabled input. Native
`ComponentWindow` routes this automatically using its existing wait-set loop.

## Verification

```sh
cargo test --manifest-path runtime/dunit-component/Cargo.toml
cargo test --manifest-path runtime/dunit/Cargo.toml --features gui
python3 tools/test_sdk_gui_fixture.py autoscroll --accel kvm
python3 tools/test_sdk_gui_fixture.py autoscroll --reuse-image --accel tcg
```

The native fixture requires multiple separate timeout returns with changed
offsets and selections at a stationary pointer, both Textarea axes and single
Input, acknowledged button releases followed by 250ms of unchanged state,
caret reveal and exact LF text restoration through edit/undo.

Validation on 30 October 2026: 83 component, 54 renderer, 35 widget, 21
layout/UI, 13 native SDK, 24 Python tooling and 111 DWM host checks passed
(341 checks total). The native fixture passed 20/20 required markers in both
KVM and TCG, no forbidden markers and guest exit 0.
Regressions passed two-axis Scroll/Textarea 28/28, multiline input 19/19,
single-line input 20/20, theme 7/7 with exact dark/light/restored framebuffer
pixels, and GUI smoke 33/33. ABI checks remain synchronized at 89 syscalls,
25 errno and 7 rights.
