# SDK push buttons (M7.4 / M7.6 first group)

`View::button(key, label, message)` creates a native SDK push button. The SDK
owns its interaction state and sends `message` through `Component::update` on
activation. Applications do not perform hit testing or issue gui-v1 messages.
The label is also the accessibility name; `accessibility_role()` is `button`.
These metadata prepare future accessibility integration, not a screen reader.

```rust
View::button("save", "Save", Msg::Save).disabled(self.saving)
```

`ComponentWindow::next_event` dispatches pointer and keyboard input before
returning the original native event. Do not activate a button again in the
application's event handler. A root shortcut remains available when no button
consumes its key. The pure `Application` exposes the same keyboard dispatcher
and `pointer(event, target_key)`; `Renderer::hit_key` resolves pointer coordinates
using the most recent layout and the existing rectangular ancestor clips.
Only the primary pointer button is dispatched by the native adapter.

## State and identity

- Move supplies `:hover`; Down focuses and supplies `:active`.
- Up on the same still-pressed button activates exactly once and clears active.
- Moving away or leaving the client permanently cancels the current press.
  Returning before release does not rearm it. Escape also cancels a press.
- Tab / Shift+Tab follow current document order, wrap, and skip disabled buttons.
  Clicking background clears focus. Space / Enter activate the focused button.
- Disabled buttons never hover, press, focus or activate.
- `:focus`, `:hover`, `:active` and `:disabled` are resolved through shared DSS.
  `assets/dwm/sdk/base.dss` uses profile tokens for every appearance value.
  `View::interaction()` provides read-only state for diagnostics and tests.

After each valid `update`, buttons with the same global stable key and kind
retain interaction state, including across reordering or label changes. Removed
keys are discarded. Changing kind or disabling resets the state. Re-enabling
or reinserting a removed node starts fresh. New activation messages always come
from the new view, not a retained copy of the previous model's callback.
Resize/theme changes preserve the application view and its state. Hit testing
uses the new layout after repaint. Style changes use existing diff/damage and
DSS motion; no separate widget painting pipeline is introduced.

## Current limits

This slice implements ordinary push buttons. Toggle/checkbox/radio/switch,
text fields, scrolling, nested lifecycle, mnemonics, event bubbling and system
clipboard remain open roadmap work.

The current native control channel forwards cooked key presses, including
repeat, and omits key releases. Keyboard activation therefore occurs per
Space/Enter event; a held key cannot expose an independent pressed/released
visual lifecycle yet. Pointer hold does expose `:active`. Adding keyboard-up
wire events is a separate protocol decision; this slice changes no ABI or wire
format. Leaving a window cancels pointer press but preserves keyboard focus;
the existing channel has no separate keyboard-focus-out event.

## Verification

```sh
cargo test --manifest-path runtime/dunit-component/Cargo.toml
cargo test --manifest-path runtime/dunit/Cargo.toml --features gui
python3 tools/test_sdk_gui_fixture.py buttons --accel kvm
```

Seven host tests cover click/release, cancellation, keyboard/focus traversal,
disabled and removed/retyped nodes, stable keys across reorder/resize,
accessibility metadata, layout hit testing, DSS state pixels and partial repaint
versus a fresh full render.

The QEMU fixture uses real QMP keyboard and pointer input through
`tools/qemu_test.py`: Tab, Space, reordered-view focus, disabled skip, Enter,
maximize with retained focus, pointer hold, leave/return cancellation and a new
click. It requires every intermediate assertion, `[WIDGETS] buttons ok`, exit 0
and absence of panic/failure markers. Validation on 28 October 2026 passed the
full 13-marker contract in KVM and TCG. The existing component contract passed
all 12 markers, including exact 15,360-pixel partial presentations, and the
desktop smoke passed all 33 markers. Forbidden markers were absent. M7.6
remains open beyond this first group.
