# Controlled horizontal slider (M7.6)

```rust
View::slider("volume", "Volume", model.volume,
    SliderOptions::new(0, 100, 10), Msg::Volume)
```

The label supplies the accessibility name; put a separate Text node beside the
control for a visible label/value. `slider_range()` exposes min/max/step/value,
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

This first slider is horizontal. Vertical orientation, wheel-based value
changes, tick marks, tooltips and keyboard key-up visuals are not included.
It adds no kernel syscall or GUI wire format.

## Shared appearance and drawing

DSS owns slider-thumb-size, slider-track-height, slider-min-track,
slider-track-color, slider-fill-color and slider-thumb-color. The bundled
light/dark profiles supply geometry tokens; base.dss supplies hover, active,
focus and disabled rules. The generic DUI measurer retains its existing
unthemed minimum-size defaults for compatibility.

Paint and pointer mapping share SliderGeometry, including border, padding and
thumb travel. Typed model fractions are immutable paint data, separate from
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
removed or weakened. This completes the horizontal slider slice; spinbox,
progress and busy indicator remain separate M7.6 work.
