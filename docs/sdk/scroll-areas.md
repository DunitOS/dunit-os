# SDK vertical scroll areas (M7.6)

`View::scroll(key, label, children)` lays out children vertically at natural
height inside a bounded viewport. It is a focusable accessibility `region`
with the supplied label. Its default `height: Grow(1)` shares the remaining
column height with other growing children. Fixed and percentage heights are
also supported. The layout engine derives flex weight from typed main-axis
`Sizing::Grow` when there is no explicit `Attrs::grow` weight.

```rust
View::scroll("actions", "Actions", rows)
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
- Up/Down move by the themed step; PageUp/PageDown by one viewport; Home/End
  reach the limits. Native text editing and radio navigation have priority;
  remaining raw scroll keys bubble through the focused node's ancestors.
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
anchored drag. The text renderer uses the same axis for fitting and wheel
clamping; it retains logical LF editing and caret behavior.

## Appearance and limits

Shared dark/light DSS defines `scroll-step` (content pixels for Scroll),
`scrollbar-width`, `scrollbar-min-thumb`, `scrollbar-radius`, `scrollbar-track`
and `scrollbar-thumb`. Dimensions/steps must be finite and within 0..1024;
colors must be valid colors. A zero step disables line/wheel scrolling; zero
width hides the bar. Keyboard paging remains available. The existing textarea
`scroll-step` is in logical lines, as documented for text input.

A fixed themed gutter is reserved inside the scroll border and padding even
when content fits, avoiding reflow feedback as the scrollbar appears. The bar
hides automatically when there is no vertical overflow. This is not timed
fade/autohide. Horizontal overflow is clipped; horizontal scrolling/bars,
scrollbars inside Textarea, virtualized rows and timed drag edge autoscroll are
separate roadmap work. This container lays out its complete bounded View tree;
it does not claim the 10,000-row virtualization criterion.

`Renderer::scroll_offset`, `scroll_viewport`, `scroll_track`, `scroll_thumb`,
`scroll_by`, `scroll_wheel`, `scroll_key` and `scroll_pointer` expose the pure
path. `ComponentWindow` exposes offset/viewport/thumb diagnostics and routes
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
