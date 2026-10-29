# SDK scroll areas (M7.6)

`View::scroll(key, label, children)` lays out children vertically at natural
height inside a bounded viewport. It is a focusable accessibility `region`
with the supplied label. Its default `height: Grow(1)` shares the remaining
column height with other growing children. Fixed and percentage heights are
also supported. The default remains vertical for compatibility;
`.scroll_axes(ScrollAxes::Horizontal)` or `.scroll_axes(ScrollAxes::Both)` enables
other axes explicitly. Children still stack vertically, and can contain wide
rows, grids or other content. The layout engine derives flex weight from typed main-axis
`Sizing::Grow` when there is no explicit `Attrs::grow` weight.

```rust
View::scroll("actions", "Actions", rows).scroll_axes(ScrollAxes::Both)
```

Viewport position is transient SDK state. The application owns child contents
and messages; it does not duplicate offset, thumb geometry or hit testing.
Offsets follow the stable key while the node remains a Scroll. Reorder, resize
and theme reload retain the offset, clamped to the new content extent.
Removal, retype or disabled state discards it. Disabling a scroll viewport
blocks its own scrolling; it does not recursively disable its children.

## Behavior

- Wheel events bubble from the deepest hit node to the nearest viewport that
  can move. At a boundary they continue to an outer viewport. A textarea gets
  first chance to scroll its own document. Negative native deltas move upward.
  Without a pointer target the native adapter uses the focused node.
- Tab traverses the region and its controls. A new focused descendant is
  revealed, from inner viewports outward. Resizing its viewport also reveals
  it. Manual scrolling survives no-op model rebuilds and color-only theme
  reloads. This does not add global focus/keyboard notification to gui-v1.
- Up/Down move vertically by the themed step; Left/Right move horizontally.
  PageUp/PageDown page vertically; Home/End reach the vertical limits. Axis-aware
  pure APIs support line/page/end commands on either axis. Native text editing and radio navigation have priority;
  remaining raw scroll keys bubble through the focused node's ancestors. The
  current native wheel channel reports only vertical deltas; horizontal wheel
  routing is available through the pure axis-aware API. No gui-v1 format changed.
- Clicking a track pages toward the pointer. Dragging the thumb preserves the
  grab point, clamps at both ends and continues outside the track within the
  client. Pointer Leave permanently cancels capture. Text-selection capture
  crossing a scrollbar is preserved. Content removal, disabled state or a
  disappearing scrollbar also cancels thumb capture.
- Paint and hit testing use the same translated child rectangles and ancestor
  viewport clipping. Scrollbars paint after their content; later siblings keep
  normal paint/hit precedence. The last painted snapshot stays immutable while
  interaction updates the controller, so incremental diff sees the change.

`dunit_widgets::ScrollAxis` supplies finite extent/offset handling, minimal
reveal, line/page/end commands, proportional/minimum thumb geometry and
anchored drag. The text renderer uses the same axis for fitting, wheel and scrollbar
clamping; it retains logical LF editing and caret behavior. Clicking or dragging
a text scrollbar, including the reserved corner or hidden-bar gutter, focuses
the field without placing the caret or starting text selection. The current
viewport is pinned before scrollbar focus, so its thumb cannot jump to the
old caret. Undo/redo history is unaffected; the next text caret action reveals
the insertion point. Blink/no-op updates retain manual viewport offsets.

## Appearance and limits

Shared dark/light DSS defines `scroll-step` (content pixels for Scroll),
`scrollbar-width`, `scrollbar-min-thumb`, `scrollbar-radius`, `scrollbar-track`
and `scrollbar-thumb`. Dimensions/steps must be finite and within 0..1024;
colors must be valid colors. A zero step disables line/wheel scrolling; zero
width hides the bar. Keyboard paging remains available. The existing textarea
vertical `scroll-step` is in logical lines; its horizontal API step is
`font-size * scroll-step` pixels.

A fixed themed gutter is reserved inside the scroll border and padding even
when content fits, avoiding reflow feedback as the scrollbar appears. Each bar
hides automatically when its axis has no overflow. This is not timed
fade/autohide. Each enabled axis reserves its own gutter. Both Textarea axes
use the shared theme by default; zero scrollbar-width hides the bars and removes
the gutters. With a collapsed content viewport the bars are suppressed to avoid
painting over the field border. Timed auto-hide, virtualized rows and timed drag
edge autoscroll remain separate roadmap work. This container lays out its complete bounded View tree;
it does not claim the 10,000-row virtualization criterion.

`Renderer::scroll_offset`, `scroll_viewport`, `scroll_track`, `scroll_thumb`,
`scroll_by`, `scroll_wheel`, `scroll_key` and `scroll_pointer` expose the pure
path. The `*_x` diagnostics and `scroll_by_x` address the horizontal axis;
`scroll_wheel_axis` and `scroll_key_axis` take `ScrollDirection`. Textarea uses
the same controller API and bar geometry. Text editing keys keep priority over
viewport navigation. `ComponentWindow` exposes offset/viewport/thumb diagnostics and routes
native events automatically.

## Verification

```sh
cargo test --manifest-path runtime/dunit-ui/Cargo.toml
cargo test --manifest-path runtime/dunit-widgets/Cargo.toml
cargo test --manifest-path runtime/dunit-render/Cargo.toml
cargo test --manifest-path runtime/dunit-component/Cargo.toml
python3 tools/test_sdk_gui_fixture.py scroll --accel kvm
python3 tools/test_sdk_gui_fixture.py scroll --reuse-image --accel tcg
```

The native fixture has 19 required markers covering keyboard focus/navigation,
real wheel input, track paging, thumb drag, activation of the last scrolled
button, no-op retention, resize, shrink/expand, `[SCROLL] ok` and guest exit 0.
All guest launches use `tools/qemu_test.py`.

The shared marker harness accepts an optional `after` boundary on a forbidden
marker; that boundary itself becomes required. Scroll's `USER-FAULT` check
covers the complete post-terminal phase: boot smoke intentionally faults
isolated test children before the terminal. Panic and application/compositor
failures remain forbidden globally. Host tests prove faults after the boundary
fail and a missing boundary also fails; existing global budgets are unchanged.

Validation on 29 October 2026: 21 layout/UI, 35 widget, 47 renderer, 67
component, 13 native SDK, 24 Python tooling and 111 DWM host checks passed.
Scroll passed 19/19 required markers in KVM and TCG, no forbidden markers,
and guest exit 0. Regressions passed multiline 19/19, single-line 20/20,
checkables 18/18, theme 7/7 plus exact dark/light/restored framebuffer pixels,
and GUI smoke 33/33. The scroll fixture screenshot was inspected. ABI remains
in sync at 89 syscalls, 25 errno and 7 rights.

Two-axis and Textarea contract (30 October 2026):

```sh
python3 tools/test_sdk_gui_fixture.py scroll_xy --accel kvm
python3 tools/test_sdk_gui_fixture.py scroll_xy --reuse-image --accel tcg
```

The 28-marker contract requires successful desktop config import as well as
keyboard navigation, real track Down and thumb Move events on both axes,
acknowledged releases between gestures,
unchanged selection and text during Textarea gestures/blink, caret reveal,
insertion/undo of the original LF document, shrink, resize and guest exit 0.
Host regressions cover nested axis bubbling, two-axis focus reveal, immutable
paint snapshots, nonfocused thumb anchoring, inactive gutters and tiny fields.
Line, wheel and bar requests share pending offsets; horizontal and vertical
requests compose in either order before the next frame.

Validation on 30 October 2026: 76 component, 54 renderer, 21 layout/UI,
35 widget, 13 native SDK, 24 Python tooling and 111 DWM host checks passed
(334 checks total). Two-axis Scroll/Textarea passed 28/28 required markers
in both KVM and TCG with no forbidden markers and guest exit 0. Regressions
passed vertical Scroll 19/19, multiline 19/19, single-line 20/20, checkables
18/18, theme 7/7 plus exact dark/light/restored framebuffer pixels, and GUI
smoke 33/33. The two-axis screenshot was inspected. ABI remains in sync at
89 syscalls, 25 errno and 7 rights.
