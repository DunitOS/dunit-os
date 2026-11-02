# Integer spinboxes

`View::spinbox(key, label, value, SpinboxOptions::new(min, max, step), on_change)`
creates a controlled, single-line integer editor with two step buttons. The model
owns the committed `i64`; the SDK retains draft text, caret, selection and undo
history. No syscall or gui-v1 change is required.

```rust
View::spinbox("quantity", "Quantity", self.quantity,
    SpinboxOptions::new(-100, 100, 10), Message::Quantity)
```

## Values and editing

The range uses the same validated `SliderOptions` arithmetic as sliders: positive
step, grid anchored at `min`, inclusive endpoints, nearest-grid rounding (ties
upwards), clamping, and overflow-safe intermediate calculations. `options.range`
also exposes `page_step`. Off-grid model values and invalid configurations fail
view validation with `UiError::InvalidSpinbox` before dispatch or painting.

Typing accepts ASCII decimal digits and one optional leading sign. Empty and
sign-only drafts are allowed; other characters are rejected atomically, retaining
selection and undo history. Drafts contain at most 20 characters, enough for every
signed `i64`. Commit parses through `i128`, so typed values outside `i64` clamp
correctly rather than wrapping.

- Enter commits; Escape restores the model value.
- Tab/Shift+Tab and a pointer Down on another target commit before moving focus.
- Up/Down step from the snapped draft. PageUp/PageDown use `page_step`.
- Home/End move the caret. Selection, mouse dragging, Ctrl+A and undo/redo use the
  common input editor.
- The upper `+` and lower `−` button each step on release in the same button.
  Moving outside that half cancels the press permanently. Leave, Escape, Tab,
  disabling, removal, or a model value/range change also cancel capture.

A commit or step dispatches at most one `fn(i64) -> Message`, and only for a value
that differs from the model. The next view accepts or rejects it; the editor then
shows the actual model value and starts a fresh undo history. Unrelated updates
preserve an unfinished draft. External value/options changes replace it.

`options.read_only` allows focus and text selection, but rejects edits and steps.
Disabled controls skip focus traversal. `accessibility_role()` is `spinbutton`;
`accessibility_label()` returns the supplied label and `spinbox_range()` exposes
min/max/step/value/read-only metadata. Metadata describes the committed value.

## Rendering and validation

The shared input renderer reserves a DSS-sized button gutter. Painting and hit
testing use the same rectangles; selection, caret and horizontal scrolling stay
inside the remaining text viewport. Dark/light profiles supply button width,
mark size, stroke and colors. Buttons are muted at range limits and when inert.

Host coverage checks drafts, clamping, validation, model rejection, focus,
readonly/disabled, capture cancellation, pixels/damage and resize. The native
`tools/test_sdk_gui_fixture.py spinbox --accel tcg` scenario verifies real key and
mouse input, cancellation, readonly, disabled traversal, ordinary button
activation and resize in one VM, through `tools/qemu_test.py`.

This is an integer control. Decimal formatting, wheel stepping and mouse-hold
repeat are outside this increment. Window deactivation does not commit a draft.
With negotiated [input lifecycle](m7-6-input-extension-proposal.md), focus-out
cancels transient capture and hides the caret while preserving the draft; focus-in
restores a still-valid focused control. Legacy peers receive no such notification.
Focus changes inside the component commit as described above.
