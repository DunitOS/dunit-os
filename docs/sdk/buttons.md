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

The compositor receives ordered device records through
[`ReadPointerEvent`](pointer-events-proposal.md), so a complete short click
between two rendered frames remains a Down followed by Up. Stream loss or seat
handover cancels capture through Leave before resynchronising; it never creates
an activation from an already-held button. The pointer queue itself does not
change client control-channel bytes. The
separately negotiated [input lifecycle](m7-6-input-extension-proposal.md) adds
keyboard release/focus semantics while preserving legacy clients.

## State and identity

- Move supplies `:hover`; Down focuses and supplies `:active`.
- Up on the same still-pressed button activates exactly once and clears active.
- Moving away or leaving the client permanently cancels the current press.
  Returning before release does not rearm it. Escape also cancels a press.
- Tab / Shift+Tab follow the configured stable [focus order](focus-order.md), wrap,
  and skip disabled buttons. Clicking background clears focus. Negotiated native
  Space / Enter arms the focused button on make and activates once on release;
  repeats remain held. Escape, focus loss and invalidated targets cancel it.
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

Ordinary push buttons are complemented by [checkable controls](checkable-controls.md),
text fields, scroll regions, mnemonics and [event routing](event-routing.md).
System clipboard and full application migration remain separate roadmap work.

Native `Window`/`ComponentWindow` negotiate `FEATURE_INPUT_LIFECYCLE`. With that
feature, Space/Enter has a real held `:active` lifecycle and one activation on
matching key-up. A stray release or release after cancellation cannot activate.
Keyboard focus-out clears transient gestures and focused/hovered appearances,
closes menus/tooltips and hides input carets. Focus-in restores a still-enabled
stable focus owner in scope without activating it.

Legacy peers and direct pure `Application::key` calls retain immediate cooked-key
activation, including their historical repeat behavior. These paths do not
pretend to receive releases or inter-window focus events. Pure host tests use
`key_press`/`key_release` and `window_focus` to exercise the negotiated lifecycle.
The accepted [gui-v1 contract](m7-6-input-extension-proposal.md) changes no syscall.
Correct native pointer modifier snapshots still await a
[separate queue decision](pointer-modifier-snapshot-proposal.md).

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

The `lifecycle` native fixture additionally covers a held/repeated Space, matching
release, held Enter cancelled by Escape, focus loss during an armed press, focus
restoration and menu dismissal, alongside a legacy peer. Its result belongs to
the current validation report, not the historical 28 October results above.
