# Controlled sliders (M7.6)

```rust
View::slider("volume", "Volume", model.volume,
    SliderOptions::new(0, 100, 10), Msg::Volume)
```

The label supplies the accessibility name; put a separate Text node beside the
control for a visible label/value. `slider_range()` exposes min/max/step/value
and `SliderOrientation` (Horizontal or Vertical),
and `accessibility_role()` is `slider`. The model owns the value; the callback
receives a proposed integer and rebuilding the view accepts or rejects it.
No speculative value is retained when the model rejects a change.

`SliderOptions` has min/max/step/page_step. Values must lie on a step from min,
or equal max; max remains a reachable endpoint when the span is not divisible
by step. Min may equal max. Invalid/reversed ranges, nonpositive steps, invalid
page steps and off-grid model values fail view validation with InvalidSlider.
The constructor chooses a page step up to ten normal steps, without overflow;
explicit page_step must be a positive multiple of step. Keyboard arithmetic
uses i128 intermediates over the full i64 range. Pointer selection is quantised
by the track's pixel/floating-point geometry, then snapped to the integer grid.

## Input and identity

- Left/Down decrement; Right/Up increment. Home/End select bounds.
  PageUp/PageDown apply page_step. Clamping does not dispatch duplicate changes.
- Track click proposes the nearest step immediately; grabbing the thumb keeps
  its pointer offset, so it does not jump under the pointer.
- Drag capture continues outside the widget inside the client window, clamped
  to the range. Up releases it. Leave, Escape, focus change, disable/removal or
  range changes cancel capture. Already accepted values are retained on cancel.
- Tab/Shift+Tab include sliders and skip disabled controls. Disabled sliders
  cannot focus, capture or propose a value. Reordering with a stable key keeps
  focus/capture; range changes cancel the gesture. Resize uses the new geometry.
- Existing text selection and scrollbar captures take precedence. The native
  adapter routes events; application code does not perform slider hit testing.

`View::vertical_slider` takes the same arguments as `View::slider`. The minimum
is at the bottom, the maximum at the top; the filled part grows upwards. The
canonical element tag is `VerticalSlider`, with the same theme tokens and states.
Its intrinsic minimum track length is vertical. Up/Right increase and Down/Left
decrease, with the same bounds and page commands in either orientation.
Changing a stable key between the two element kinds resets transient focus and
capture; rebuilding or resizing the same kind preserves focus.

Wheel-based value changes, tick marks, tooltips and a separate held-arrow slider
visual are not included. Arrow/page value changes remain key-make actions;
negotiated key-up does not repeat a value change. Negotiated window focus-out
cancels capture without rolling back an already accepted value, and focus-in can
restore its stable focus owner. Legacy peers retain their existing behavior.
The slider API adds no syscall; its window adapter shares the
[approved gui-v1 lifecycle](m7-6-input-extension-proposal.md).

## Shared appearance and drawing

DSS owns slider-thumb-size, slider-track-height, slider-min-track,
slider-track-color, slider-fill-color and slider-thumb-color. The bundled
light/dark profiles supply geometry tokens; base.dss supplies hover, active,
focus and disabled rules. The generic DUI measurer retains its existing
unthemed minimum-size defaults for compatibility.

Paint and pointer mapping share SliderGeometry, including border, padding and
thumb travel. `SliderGeometry::coordinate(x, y)` chooses the primary axis for
`Application::slider_pointer`; it returns x for horizontal and y for vertical.
`fraction`, `grab` and `fill` use this same oriented geometry. Typed model fractions are immutable paint data, separate from
DSS policy. Value changes participate in retained damage; old thumb/fill pixels
are cleared, ancestor clips and scroll translation remain effective. Existing
low-level painter entry points remain compatible.

## Verification

Eight host scenarios cover validation, i64 extremes and endpoint arithmetic,
keyboard/focus/accessibility, capture and cancellation, rejected changes,
pixel/damage/resize equivalence and scrolled geometry/hits. Existing widgets,
renderer and component host suites also run as cheap regressions.

One native QEMU fixture covers arrows, Home/End/PageUp/PageDown, no duplicate
endpoint callback, track click, captured drag/clamp/release, disabled Tab skip,
ordinary button activation and resize/focus. The standard fixture wrapper
builds one image and runs the whole scenario in one VM:

```sh
python3 tools/test_sdk_gui_fixture.py slider --accel tcg
```

The marker contract is tools/sdk_slider_markers.json. No existing marker is
removed or weakened. Integer spinboxes have their own [contract](spinboxes.md);
[progress and busy indicators](progress.md) have a separate passive API.

Six additional host scenarios cover vertical direction, asymmetric padding,
primary-axis thumb offset, rejected proposals, cancellation (including orientation
changes), dark/light rendering and damage, resize and scroll clipping. Generic
widget measurement checks preserve the unthemed horizontal defaults and transpose
them for vertical sliders.

```sh
python3 tools/test_sdk_gui_fixture.py vertical_slider --accel tcg
```

This single native scenario checks Up, Home/End and pages, track click, upward
captured drag/clamp, thumb grab without a jump, raw Escape cancellation while the
mouse is held, cancelled movement/release, disabled Tab skip, ordinary button
activation and resize/focus. Its contract is
`tools/sdk_vertical_slider_markers.json`.
