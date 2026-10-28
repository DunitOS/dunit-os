# Checkable SDK controls (M7.6)

The typed SDK now provides `View::toggle_button`, `View::checkbox`,
`View::switch` and `View::radio`. They use the same pointer press/cancellation,
keyboard activation, stable keys and native window adapter as [push buttons](buttons.md).
There is no second application hit-test or painting loop.

```rust
View::checkbox("autosave", "Autosave", self.autosave, Msg::ToggleAutosave)
View::switch("notifications", "Notifications", self.notifications, Msg::ToggleNotifications)
View::toggle_button("bold", "Bold", self.bold, Msg::ToggleBold)
View::radio("compact", "Compact", "density", self.compact, Msg::Compact)
View::radio("comfortable", "Comfortable", "density", !self.compact, Msg::Comfortable)
```

`checked` belongs to the component model. Activation sends the supplied message
through `update`; the next `view()` supplies the new value. A model may decline
an activation without the SDK toggling hidden state. Transient hover, focus and
press survive a rebuild with the same key, kind and radio group; checked always
comes from the new view. Changing kind/group, removal and disabled transitions
reset transient interaction. Disabled checked controls keep their model value
and checked appearance.

All controls activate on primary-pointer release inside the same armed node, or
on a focused Space/Enter cooked key event. Moving away, leaving the client or
Escape cancels a pointer press. Returning does not rearm it. Keyboard key-up and
external focus-out limits are unchanged from the push-button contract.

## Radio groups

A radio needs a nonblank group name, and each group can have at most one checked
member, including disabled members. Invalid views return `UiError::InvalidRadioGroup`
before replacing the previous valid view. Zero selected members is permitted.
The model's selection handler must update all members consistently.

A group contributes one Tab stop: its focused enabled member, otherwise its
checked enabled member, otherwise its first enabled member. Shift+Tab traverses
in reverse. Arrow keys move and select within the focused group in document
order, wrap, and skip disabled members. Independent groups do not interfere.
Activating a checked radio consumes the event without sending another selection
message. Arrow movement also reads selection back from the model; a model that
rejects the change may leave focus on an unchecked member.

`Application::navigate(Navigation::{Left,Right,Up,Down})` supplies the pure host
path. The native adapter maps the existing unmodified, uncooked arrow scancodes
in IN_KEY; no ABI or wire-format changes are required.

## Appearance and metadata

`Interaction::checked` supplies the DSS `:checked` state, including while disabled.
The shared base stylesheet and dark/light profiles supply checked, pressed,
hovered, focused and disabled appearances. A reserved outer border makes focus
visible on a checked switch without changing layout. Labels and indicators
remain vertically centered as control height changes. Checkbox has a checkmark, Radio a
circular dot, Switch a track and a moving thumb; ToggleButton changes its box.
Every color and configurable dimension comes from DSS tokens.

The indicator properties are `indicator-width`, `indicator-height`,
`indicator-gap`, `indicator-border-width`, `indicator-radius`,
`indicator-mark-size`, `indicator-inset`, `indicator-position`,
`indicator-background`, `indicator-border-color` and `indicator-mark-color`.
Measurement and painting share indicator geometry and omit a trailing gap when
there is no label. Theme validation rejects invalid types/ranges. Indicators
and labels are contained within the control box, including after resize.
Existing damage tracking repaints checked-state changes without a separate
widget renderer. Custom DSS motion properties remain subject to the documented
[supported motion profile](motion.md); indicator transitions are not added here.

Accessibility metadata expose each control's role and label, `accessibility_checked`
for checkbox/radio/switch, and `accessibility_pressed` for a toggle button.
These are metadata for future accessibility integration, not a screen reader.

## Verification

```sh
cargo test --manifest-path runtime/dunit-component/Cargo.toml
cargo test --manifest-path runtime/dunit-render/Cargo.toml
cargo test --manifest-path runtime/dunit-widgets/Cargo.toml
python3 tools/test_sdk_gui_fixture.py checks --accel kvm
python3 tools/test_sdk_gui_fixture.py checks --reuse-image --accel tcg
```

Host tests exercise model ownership (including declined activation), all four
controls, cancellation/disabled cases, radio exclusivity/arrow traversal/Tab,
reorder/retype/regroup, accessibility metadata, dark/light checked pixels,
indicator geometry/clipping and incremental pixels against a full repaint.
The native fixture requires keyboard and pointer activation of all four kinds,
radio wrap and disabled skip, group Tab navigation, resize with retained values,
disabled pointer rejection, `[CHECKS] ok` and guest exit 0. Its full contract
contains 18 required markers and forbids panic/failure markers.

This completes the next button family slice. Text fields, scrolling, other
widget families, application migration and the key-release protocol decision
remain open roadmap work.

Validation on 28 October 2026: 41 component, 28 renderer, 12 widget, 18 style,
13 native SDK and 16 Python tooling checks passed. Final checkable runs passed
18/18 required markers in both KVM and TCG; the push-button regression passed
13/13, theme regression 7/7 plus exact dark/light/restored framebuffer pixels,
and GUI smoke 33/33. Forbidden markers were absent. ABI generation remained
in sync at 89 syscalls, 25 errno and 7 rights. Real screenshots were inspected
after the alignment/focus fixes.
