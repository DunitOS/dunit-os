# Workspace controls (M7.6)

The typed SDK provides tabs, split panes, toolbars, status bars, modal dialogs
and message boxes. They use the existing component model, DSS, CPU painter,
seat input and wait-set loop. No kernel or gui-v1 change is required.

## Tabs and command bars

```rust
View::tabs("documents", "Documents", self.active, vec![
    Tab::new("edit", "Editor", Msg::Tab("edit"), editor_view()),
    Tab::new("preview", "Preview", Msg::Tab("preview"), preview_view()),
])
View::toolbar("tools", "Commands", vec![
    View::button("save", "Save", Msg::Save),
    View::toolbar_separator("separator"),
    View::button("open", "Open", Msg::Open),
])
View::status_bar("status", "Operation status", self.status.as_str())
```

Tabs are controlled: selecting a header dispatches its message, and the next
model view accepts or rejects the selected key. Disabled headers are skipped.
Left/Right wrap over enabled tabs; Home/End select endpoints. Ctrl+Tab and
Ctrl+Shift+Tab switch the nearest active tab set, including from page controls.
There is one Tab stop for the tab header group, followed by active page controls.
Already-selected tabs do not dispatch duplicate activations.

Only the selected page is mounted. Its model values survive switching; transient
editor selection/undo/caret state is recreated when an inactive page remounts.
No hidden page participates in input, timers, layout or painting. Page IDs must
be nonempty and unique, and the selected ID must exist. Empty tab sets and bad
inactive-page descriptions fail validation. Generated keys use
`<tabs-key>/tab/<page-id>`, `<tabs-key>/headers` and
`<tabs-key>/panel/<page-id>` and must not collide with application keys.

A toolbar has one remembered Tab stop. Left/Right and Home/End move focus among
enabled commands without activating them; Space/Enter or a click activates the
focused command normally. It accepts direct button/checkable/menu/select
commands and passive labels/separators. Nested focusable controls and editors
are rejected: use a regular Row for a form/search bar that owns its caret keys.
Status bars and separators are passive and never dispatch input messages.

Roles include group, tablist, tab, tabpanel, toolbar, status and separator.
`selected_tab`, `accessibility_selected`, `accessibility_controls` (for the
mounted selected panel), `accessibility_labelled_by` and orientation metadata
are available for a future accessibility bridge.

## Controlled split panes

```rust
View::splitter("panes", "Pane sizes", self.ratio,
    SplitterOptions::new(SplitAxis::Horizontal), left_view(), right_view(),
    Msg::Ratio)
```

The ratio is an integer in permille (0..1000). `SplitterOptions::range` reuses
validated overflow-safe SliderOptions for logical bounds, step and page step;
the default step is 10. Horizontal places panes left/right, Vertical top/bottom.
The generated `<split-key>/handle` is a focusable separator with a supplied label,
orientation metadata and `splitter_value()`.

Dragging preserves the initial pointer offset, continues outside the handle
inside the client, and clamps at endpoints. Left/Up decrease, Right/Down
increase; Home/End and PageUp/PageDown use the configured range. Escape, Leave,
removal, disabling or axis/range changes cancel capture. Resize uses the new
geometry. The model accepts or rejects proposals; unchanged values do not
redispatch. Disabling the splitter disables its generated handle while keeping
pane contents available.

Paint layout, hit testing and pointer mapping share SplitGeometry. Pane primary
min/max layout constraints bound the visual divider; impossible minima shrink
proportionally to fit. The model still owns the requested ratio, so its logical
value may differ from the visually constrained position in a very small client.
If both pane maxima cannot fill the area, remaining space is left unused.
Nested splitters arrange parents before descendants; no application hit-test or
manual painting is needed.

The UI layout layer now exposes validated subtree measurement and rearrangement
for these compound controls. Invalid IDs/nonfinite rectangles are rejected
without changing the prior layout. Scroll capture is cancelled when modal/popup
ownership changes or a fresh splitter gesture begins.

## Modal dialogs and message boxes

```rust
let dialog = self.show_dialog.then(|| View::dialog(
    "settings", "Settings", settings_body(),
    vec![DialogAction::new("accept", "Save", Msg::Accept),
         DialogAction::new("cancel", "Cancel", Msg::Cancel)],
    Some("accept"), Some(Msg::Cancel)));
View::modal("window", main_view(), dialog)
```

`View::modal` is a top-level client layer: the base view, a themed scrim and one
centered dialog. `View::dialog` can also be the sole root view of a dedicated
client window. Nested modal hosts, dialogs outside their host and multiple
simultaneously mounted dialogs fail validation. The default action must refer
to an activation-capable descendant.

Opening captures the opener and cancels other gestures/popups/tooltips. Focus
starts at the enabled default action, otherwise the first eligible descendant.
Tab/Shift+Tab stay inside the dialog. Background clicks, menu mnemonics, tab
switching and root shortcuts cannot activate the base UI. Menus and input fields
inside the dialog retain their own routing priority.

Enter activates a focused action, or the default action from a single-line
field; multiline editing and spinbox commit retain their existing Enter rules.
Escape first cancels an open menu or splitter gesture, then sends the dialog's
cancel message. A dialog without a cancel message consumes Escape. Closing only
happens when the model removes the dialog; rejecting a close request leaves the
scope active. The model also owns draft/accept/cancel data semantics.

Closing restores the eligible opener. Replacing a dialog preserves the original
base opener. Resize/theme reload preserves model and focus while recomputing
bounded placement; the body viewport shrinks before actions become unreachable.
Background model updates are allowed while input is scoped to the dialog.

```rust
View::message_box("notice", "Notice", "First line\nSecond line", vec![
    DialogAction::new("ack", "OK", Msg::Acknowledge),
], Some("ack"), Some(Msg::Acknowledge))
```

Message boxes use the same dialog mechanism. Plain LF-delimited message lines
sit in a two-axis scroll viewport; long lines scroll rather than automatically
word-wrap. The viewport label derives from the supplied title. `active_dialog()`
exposes the active scope in Application and ComponentWindow. These are modal to
one client surface, not compositor-managed dialogs spanning multiple windows.

## Theme and verification

Dark/light DSS owns tab states, toolbar/status/separator appearance,
`splitter-thickness`, `dialog-width`, `message-body-height` and the scrim color.
The SDK adds no appearance literals to widget code. Normal retained invalidation,
clipping, theme reload and existing input/scroll controllers remain in use.

Fourteen new host scenarios verify controlled selection/rejection, disabled
traversal, remounting, toolbar focus, passive status, default/cancel and focus
restore, modal menu/shortcut blocking, replacement, both split orientations and
nested layout, minima/degenerate sizes, pixels/damage/resize in both themes,
validation and atomic layout failure. Existing UI/widgets/render/component
suites run as regressions.

```sh
python3 tools/test_sdk_gui_fixture.py workspace --accel tcg
```

One native showcase drives all six controls with real keys and pointer input,
checks model persistence, splitter grab/drag/clamp/release, resizes the workspace,
blocks background input while modal, edits a dialog field, verifies default and
Escape restore, then resizes and acknowledges a message box. The marker contract
is `tools/sdk_workspace_markers.json`; it captures panel/dialog/message frames.
