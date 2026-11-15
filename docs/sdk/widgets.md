# SDK controls

Controls use model-owned values, stable keys and shared DSS rendering. The SDK
handles input, focus and gestures before ComponentWindow returns the native event.
Accessibility role/name/state APIs are metadata for a future bridge.

## Push buttons

`View::button(key, label, message)` creates a native SDK push button. The SDK
owns its interaction state and sends `message` through `Component::update` on
activation. Applications do not perform hit testing or issue gui-v1 messages.
The label is also the accessibility name; `accessibility_role()` is `button`.

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
[`ReadPointerEvent`](input.md#queued-pointer-input), so a complete short click
between two rendered frames remains a Down followed by Up. Stream loss or seat
handover cancels capture through Leave before resynchronising; it never creates
an activation from an already-held button. The pointer queue itself does not
change client control-channel bytes. The
separately negotiated [input lifecycle](input.md#keyboard-and-focus-lifecycle) adds
keyboard release/focus semantics while preserving legacy clients.

### State and identity

- Move supplies `:hover`; Down focuses and supplies `:active`.
- Up on the same still-pressed button activates exactly once and clears active.
- Moving away or leaving the client permanently cancels the current press.
  Returning before release does not rearm it. Escape also cancels a press.
- Tab / Shift+Tab follow the configured stable [focus order](input.md#focus-ordering), wrap,
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

### Current limits

Ordinary push buttons are complemented by [checkable controls](widgets.md#checkable-controls),
text fields, scroll regions, mnemonics and [event routing](input.md#event-routing).
System clipboard uses the shared broker described in the desktop and transfer guide.

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
The accepted [gui-v1 contract](input.md#keyboard-and-focus-lifecycle) changes no syscall.
Native pointer modifiers use the separately
[approved enqueue-time snapshots](input.md#pointer-modifier-snapshots).

## Checkable controls

The typed SDK provides `View::toggle_button`, `View::checkbox`,
`View::switch` and `View::radio`. They use the same pointer press/cancellation,
keyboard activation, stable keys and native window adapter as [push buttons](widgets.md#push-buttons).
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

All controls activate on primary-pointer release inside the same armed node.
With negotiated native input lifecycle, focused Space/Enter arms on make and
activates once on matching key-up; repeats remain held. Escape, keyboard focus-out,
disable/removal or a changed focus owner cancels that activation. Returning focus
or a late release does not rearm it. Legacy peers and pure `Application::key`
retain immediate cooked-key activation. Pointer cancellation and all keyboard
lifecycle rules are shared with the [push-button contract](widgets.md#push-buttons).

### Radio groups

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

### Appearance and metadata

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
[supported motion profile](themes.md#appearance-transitions); indicator transitions are not added here.

Accessibility metadata expose each control's role and label, `accessibility_checked`
for checkbox/radio/switch, and `accessibility_pressed` for a toggle button.
These are metadata for future accessibility integration, not a screen reader.

## Sliders

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

### Input and identity

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
[approved gui-v1 lifecycle](input.md#keyboard-and-focus-lifecycle).

### Shared appearance and drawing

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

## Spinboxes

`View::spinbox(key, label, value, SpinboxOptions::new(min, max, step), on_change)`
creates a controlled, single-line integer editor with two step buttons. The model
owns the committed `i64`; the SDK retains draft text, caret, selection and undo
history. 

```rust
View::spinbox("quantity", "Quantity", self.quantity,
    SpinboxOptions::new(-100, 100, 10), Message::Quantity)
```

### Values and editing

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

### Rendering and validation

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
repeat are outside this control API. Window deactivation does not commit a draft.
With negotiated [input lifecycle](input.md#keyboard-and-focus-lifecycle), focus-out
cancels transient capture and hides the caret while preserving the draft; focus-in
restores a still-valid focused control. Legacy peers receive no such notification.
Focus changes inside the component commit as described above.

## Progress and busy indicators

```rust
View::progress("download", "Download", self.received,
    ProgressOptions::new(0, self.total))
View::busy("working", "Working", self.running)
```

These are passive, model-owned indicators. They never focus, capture the pointer,
activate or dispatch messages. Tab skips them; application buttons, shortcuts or
background work update the model normally. Use a separate Text node for a visible
label or percentage. `accessibility_role()` returns `progressbar`,
`accessibility_label()` returns the supplied name, and `progress_state()` exposes
either `Determinate { value, options }` or `Busy { running }`.
`accessibility_busy()` is true only for a running busy indicator; busy has no
invented numeric percentage.

### Values and appearance

`ProgressOptions { min, max }` accepts an inclusive `i64` range and values within
it. Invalid/reversed/out-of-range values fail view validation with
`UiError::InvalidProgress`. Intermediate subtraction uses `i128`, including the
full signed integer range. A zero-span range is complete. Determinate fill grows
from left to right, with no speculative interpolation or callbacks.

A running busy indicator shows a bounded segment moving left-to-right and back.
Paused busy shows the empty track. Disabled indicators retain their model state
and use the theme's muted fill; a disabled busy indicator shows a stationary
centered segment. Reduced motion and immediate untimed rendering also show a
stationary centered segment for running busy indicators.

Dark/light profiles provide all appearance and timing through DSS:
`progress-thickness`, `progress-min-track`, `progress-radius`,
`progress-track-color`, `progress-fill-color`, `busy-segment` (0..1),
`busy-duration` and `busy-interval` (milliseconds). Typed SDK theme validation
rejects wrong types, negative dimensions/times and out-of-range segment sizes.
The generic unthemed Progress measurer keeps its previous defaults.

### Sampling and retained rendering

The existing window wait-set samples busy indicators; no extra thread, syscall
or gui-v1 message is required. The default profile uses a 1200 ms round trip and
40 ms sampling interval. Multiple visible busy indicators share the earliest
requested interval with paint-property transitions. Their phase uses the
window's monotonic clock, so unrelated model updates and resize do not restart
an operation's displayed cycle. This is a visual cycle, not elapsed task time.

`Renderer::render_at` accepts an explicit clock for deterministic host frames.
`render` uses the stationary representation. The renderer exposes
`busy_interval_ms()` and `RenderStats::active_busy_indicators`; the latter counts
visible moving indicators that currently require a timer.
`ComponentWindow::is_animating()` includes these indicators.

Paused, disabled, reduced-motion, zero-duration/interval, zero/full-segment and
fully clipped indicators require no busy timer. Removing the last busy node also
stops scheduling. Visibility includes the surface, ancestor rectangles and
scroll viewports; it does not infer whether another desktop window covers the
client. `cancel_motion` cancels pending sampling; a later timed render may start
it again. Stylesheet reload rebuilds the snapshot and applies the new cadence.

Progress and busy paint snapshots participate in retained damage. Geometry
includes border/padding and the theme's track thickness; clipping, scroll
translation, old-fill clearing and resize use the common renderer. Unchanged
values/phase skip publication. Dirty pixel counts describe invalidation area,
not a measured whole-desktop frame-time budget.

## Menus and tooltips

All five controls share a client-side overlay layer. Application owns transient
popup/tooltip state; the model owns choices and action effects. ComponentWindow
routes the existing seat events and wait-set deadlines. 

### Declarative API

```rust
View::select("language", "Language", Some(self.language.as_str()), vec![
    SelectOption::new("rust", "Rust"),
    SelectOption::new("locked", "Unavailable").disabled(true),
    SelectOption::new("c", "C"),
], Message::Language)

View::menu_bar("main-menu", vec![
    View::menu("file", "File", vec![
        MenuItem::action("open", "Open", Message::Open).mnemonic('o'),
        MenuItem::separator("separator"),
        MenuItem::submenu("recent", "Recent", vec![
            MenuItem::action("recent-one", "One", Message::RecentOne),
        ]),
    ]).menu_mnemonic('f'),
])

View::button("help", "Help", Message::Help)
    .context_menu(vec![MenuItem::action("inspect", "Inspect", Message::Inspect)])
    .tooltip("This text appears after the configured hover delay.")
```

`View::menu` also works independently as a menu button. `context_menu` and
`tooltip` attach to any View and bubble from a descendant to the nearest
configured ancestor. Context menus can coexist with a menu/select trigger on
the same View. Labels are plain text; mnemonics are explicit characters, not
embedded ampersands. `MenuItem::disabled` applies to actions or whole submenus.

Option keys and sibling menu keys must be nonempty and unique. A selected key
must exist; None displays the select's supplied label. Disabled selected values
may still be displayed, but cannot be chosen again by navigation. Menu mnemonic
characters are unique within a level; Alt trigger mnemonics are unique across
the View. Invalid descriptions fail validation with InvalidPopup. Menus have a
technical bound of 16 levels and 4096 entries per description. Empty and fully
disabled menus remain dismissible and never dispatch an action.

### Selection, navigation and capture

Select is controlled. Opening a popup or moving its highlight never changes the
model. Enter or a matching mouse press/release proposes the highlighted option
through `fn(String) -> Message`; choosing the committed option dispatches nothing.
The next model view accepts or rejects a proposal. Rejection leaves the previous
value displayed, with focus restored to the trigger.

- Tab includes menu/select triggers and skips disabled controls. A menu bar has
  one Tab stop, remembers its last enabled menu, and uses Left/Right internally.
- Enter/Space or Down opens a focused trigger; Up opens from the last enabled
  menu entry (select prefers the committed enabled option). Alt+Down also opens.
- In a popup, Up/Down wrap over enabled entries, skipping separators and disabled
  items. Home/End reach the first/last enabled entry. Wheel steps the highlight
  and keeps it in view. A character cycles entries with a matching first letter;
  an explicit menu-item mnemonic activates that entry.
- Right opens a submenu. Left closes one submenu, or switches a root menu in a
  menu bar. Hover opens submenus and switches between open menu-bar triggers.
- Escape closes one level, then the root. F10 focuses the first menu-bar item,
  or closes an open menu; Alt+configured-letter opens its trigger.
- A right-button Down opens the nearest context provider at the pointer.
  Shift+F10 or the Context Menu key opens it near the focused control. Context
  menus preserve the prior control's focus/selection and restore focus on close.
- Mouse actions require Down and Up in the same enabled entry. Moving off the
  pressed entry, Leave or resize cancels the pending activation. Hover alone
  never invokes an action. Keyboard input goes to the open popup before inputs,
  spinboxes, sliders or root shortcuts.
- An outside primary Down dismisses the popup and consumes the matching Up.
  It does not activate a control underneath. Tab closes the popup and advances
  ordinary traversal. Disabled/removed/retyped owners and changed entry shapes
  or selected values cancel stale popup/capture state. Unrelated model updates
  retain the open menu; action payloads come from the current view.

A normal trigger restores focus to itself. Context providers restore the
previous focused control if it still exists and is enabled. Opening a menu
cancels text/slider/button captures; Alt/F10 focus changes commit a focused
spinbox draft before leaving it.

### Placement and painting

Overlays paint after the main tree and intercept input before underlying
controls. They use the same tree painter, font metrics and DSS cascade as the
base UI. They can overlap an ancestor scroll viewport while staying inside the
client surface. Popup hit testing uses the exact row rectangles produced by the
overlay layout. Context menus clamp to the client; anchored popups flip upwards
and submenus flip sideways when needed. Long menus window the visible rows
around the active entry, keeping keyboard/wheel choices reachable.

Dark/light DSS supplies Popup, MenuItem, MenuSeparator, MenuButton, Select,
Tooltip and TooltipText appearance and state rules. Geometry and timing come
from `menu-min-width`, `menu-row-height`, `menu-separator-height`, `menu-gap`,
`menu-submenu-mark`, `control-suffix`, `tooltip-delay`, `tooltip-gap` and
`tooltip-max-width`. Wrong types, negative metrics/times and invalid mnemonic
configurations are rejected. No widget/application drawing literals are needed.

Opening, moving and dismissing overlays invalidates their old/new visual bounds;
base changes repaint overlays over the dirty area. Resize/theme reload rebuilds
placement and pixels. Overlay backdrop blur conservatively forces a full client
rebuild when damage exists, rather than sampling a previously filtered canvas.

Custom host renderers must use `Renderer::render_application` or
`render_application_at` to include transient overlays. The existing `render(view)`
API remains an immediate base-view renderer. ComponentWindow does this
integration automatically.

### Tooltips and accessibility

Tooltip delay defaults to 600 ms in the SDK profiles. Moving within the same
anchor does not restart it. A visible tooltip needs no repeat timer; text wraps
to its themed width and is bounded by the client. It never intercepts input.
Key input, button activity, Leave, scrolling, resize or opening a menu dismisses
it. Holding primary Down prevents rearming even after a control press cancels;
release or Leave ends that gesture. Changed/disabled/removed anchors cancel the
deadline. Empty/whitespace-only tooltip text disables the hint.

`accessibility_role` exposes combobox/menubar/button; `accessibility_label` uses
the control name and `accessibility_has_popup` identifies listbox/menu providers.
`selected_option` describes the committed choice. Tooltip text is also available
as `accessibility_description` without waiting for the visual hint.
`Application::accessibility_expanded`, `popup_state` and `popup_items(prefix)`
expose runtime expansion and item key/label/role/enabled/selected/submenu metadata
for a future accessibility bridge. ComponentWindow exposes popup/tooltip state
for diagnostics. These metadata do not claim an existing OS screen-reader/IME
service.

## Lists

`View::list` adds a controlled, uniform-row-height collection to the typed SDK.
The component owns row data and stable-key selection. A single UI node represents
the collection: it does not create a View, layout box or widget per model row.
The renderer snapshots and rasterizes only rows intersecting the viewport and ancestor/display clips.
Total model storage and selection validation still scale with model length;
this is rendering virtualization, not an asynchronous/paged data provider.

```rust
View::list("files", "Documents", rows, selection,
    SelectionMode::Multiple, Msg::Selection)
    .layout(Attrs {
        width: Sizing::Grow(1.),
        height: Sizing::Grow(1.),
        ..Default::default()
    })
```

Rows are `ListItem::new(stable_key, label).disabled(bool)`. Keys must be nonempty
and unique inside the collection; they are independent of global UI node keys.
`ListSelection` contains a set of selected keys, a current key and a range anchor.
The model accepts/rejects proposed state through `fn(ListSelection) -> Message`.
No callback is dispatched for an unchanged proposal. Invalid or disabled keys
in selection fail view validation. After removing/disabling rows, the model can
call `selection.retain(&rows)` to remove stale keys. Sorting/reordering the model
preserves selection identity. No implicit sorting or rename callback is claimed.

### Input and focus

The whole list is one Tab stop. Empty/all-disabled lists remain focusable but
have no eligible selection. Up/Down skip disabled rows without wrapping;
Home/End choose endpoints; PageUp/PageDown move by the visible page's enabled
row count minus one. At endpoints navigation is consumed without duplicate
messages. Single mode always selects just the current row.

Multiple mode supports:

- plain navigation/click: replace selection and establish a range anchor;
- Shift navigation: select an inclusive range from the anchor, excluding disabled rows;
- Ctrl navigation: move current without changing selection;
- Space: toggle current and establish a new anchor;
- Ctrl+Shift navigation: add a range to the existing selection;
- Ctrl+A: select all enabled rows.

A primary pointer Down selects the hit row immediately; release does not
redispatch. Blank space and disabled rows preserve selection. Pointer geometry
comes from the last painted viewport and the same row-height/scroll offset as
painting. Native pointer events carry enqueue-time Shift/Ctrl/Alt/Super snapshots.
`Application::list_pointer(..., modifiers)` supplies the corresponding pure API.
Pointer range dragging, touch, typeahead and double-click activation are outside
this slice. The list is passive with respect to Enter; applications may bind it.

Shared focus and modal scopes apply: a dialog blocks base collection input and
closing restores the list opener. Public `list_command(command, modifiers, page)`
lets non-native adapters use the same behavior; `Application::navigate` also
routes plain Up/Down. Root shortcuts retain existing routing after list-specific
keys. There are no new syscalls or gui-v1 fields.

### Geometry, scrolling and painting

`ListGeometry` shares `visible()`, `row_at()` and `page()` between adapters,
paint snapshots and tests. A partially visible first/last row is included and
clipped. Current changes/focus entry/resize reveal the current row. Wheel and
scrollbar gestures retain manual offset until another reveal trigger. The
existing ScrollAxis controller owns clamping, wheel bubbling, thumb/track input,
auto-hidden bars and a reserved gutter. Shrinking the model clamps the offset.
Disabled lists stop input and show their content at the start.

DSS controls `list-row-height`, text/font/padding, border/focus state,
`list-selection-background`, `list-selection-color`, `list-disabled-color`,
`list-current-color`, and the shared scrollbar properties. Both SDK profiles
provide row height. Nonpositive/nonfinite row height fails layout; invalid theme
values fail transactional Theme parsing. Long row labels clip horizontally;
variable heights, word wrap and horizontal scrolling are not implemented here.

Snapshots store visible labels, indices, selection/current/disabled state and
geometry. Retained invalidation includes them, so selection/scroll/data changes
repaint correctly; unchanged frames do not repaint. `Renderer::list_visible_rows`
and `list_geometry` (also exposed by ComponentWindow) allow observing this bound.
`View` supplies listbox role, supplied label, `accessibility_multiselectable`,
`list_items` and `list_selection` for a future accessibility bridge, including
unmounted/offscreen row data. There is no platform accessibility service yet.

## Trees

`View::tree_view` extends the shared virtual collection controller with a
validated hierarchy, controlled expansion, spatial navigation and disclosures.
The existing `View::tree()` method still builds the DUI layout tree; it is not
renamed or replaced by this constructor. 

```rust
let model = TreeModel::new(vec![
    TreeItem::new("workspace", "Workspace"),
    TreeItem::new("src", "Sources").under("workspace"),
    TreeItem::new("main", "main.rs").under("src"),
]).expect("valid application model");
View::tree_view("project", "Project", model.clone(), state.clone(),
    SelectionMode::Multiple, Msg::TreeState)
    .layout(Attrs {
        width: Sizing::Grow(1.),
        height: Sizing::Grow(1.),
        ..Default::default()
    })
```

### Model and controlled state

`TreeItem` contains a stable-key ListItem and an optional stable parent key.
Parents can appear after children in the supplied vector. Roots and siblings
follow input order; visible traversal is depth-first preorder. `TreeModel::new`
rejects empty/duplicate keys, missing parents, cycles (including disconnected
cycles), and nesting beyond 64 levels. It validates hidden branches too. Empty
models are valid. The immutable model shares validated graph data through Rc,
so cloning it into the next View does not clone every node or rebuild edges.
This cache contains model data, not per-row UI nodes.

`TreeState` owns `selection: ListSelection` and `expanded: BTreeSet<String>`.
The callback receives one complete proposed state, which the model accepts or
rejects. Selection and expansion have no hidden independent controller state.
Unchanged proposals do not dispatch messages. Expanded IDs must name existing
branches; selected/current/anchor IDs must name visible enabled rows. Invalid
view state fails validation rather than silently dropping user data.

For external collapse/removal/disable updates, `state.retain(&model)`:

- removes invalid/leaf expansion keys;
- removes hidden, removed or disabled selections;
- moves a hidden current/anchor to the nearest visible enabled ancestor;
- selects that ancestor if the hidden current was selected.

A removed node has no ancestry in the new model, so its current/anchor clears.
Collapsing retains expansion flags of hidden branches, allowing their shape to
return when an ancestor reopens. Reordering preserves stable-key selection and
expansion; the shared reveal logic follows the current row's new visible index.
A disabled node disables its whole subtree, including already expanded children.
The application may still change expansion programmatically while disabled.

### Input, scope and metadata

The tree is one Tab stop. Shared collection behavior handles Up/Down,
Home/End/PageUp/PageDown, single/multiple/range selection, Shift/Ctrl navigation,
Space and Ctrl+A. Ranges and Select All include only visible enabled rows.
No hidden descendant receives input or becomes a selection endpoint.

Unmodified Right expands a collapsed branch; on an expanded branch it selects
the first enabled direct child. Left collapses an expanded branch; otherwise
it selects the visible parent. At a root/leaf endpoint, or with no current row,
spatial commands are consumed without a callback. `Application::tree_command`
exposes Expand/Collapse/Toggle; plain Left/Right also use Application::navigate.

A primary Down in the disclosure toggles expansion while preserving selection,
except for normalization of hidden descendants. Clicking the label/body uses
normal selection and does not expand. Release does not redispatch. Disclosures
use the same geometry for painting and hit testing. Native pointer and keyboard events carry their modifier snapshots; pure
adapters supply modifiers through `list_pointer`.
Double-click activation, range dragging, typeahead and lazy data loading are not
part of this slice.

Modal and popup scopes, focus restore, background shortcut blocking and disabled
input use the existing shared machinery. A tree inside a toolbar is rejected
like other non-command collection controls.

The root exposes role `tree`, supplied label and multiselectable metadata.
`tree_item(key)` reports a visible row's `treeitem` role, supplied label, parent,
1-based level/position, sibling count (including disabled siblings), expanded
state for branches, inherited disabled state, selected and current. Hidden rows
are absent from this view metadata. `tree_state()` reports controlled state.
These are descriptors for a future accessibility bridge, not an OS a11y service.

### Virtualization and DSS

ListGeometry, ScrollAxis, gutter/bar input, clipping, retained invalidation and
visible ListPaint snapshots are shared with lists. The tree does not build
layout boxes or View widgets for model rows. Visibility projection/validation
still visits model data; only rows intersecting viewport/ancestor/display clips
are retained as text paint rows and rasterized. This is rendering virtualization,
not a paged or asynchronous data source. `list_visible_rows` and `list_geometry`
on Renderer/ComponentWindow observe that bound for either collection kind.

DSS supplies the common list row height, selection/current/disabled colors,
font, padding, border/focus and scrollbar properties. Trees additionally use
`tree-indent`, `tree-toggle-width`, `tree-open-mark` and `tree-closed-mark`.
The two profiles provide indent and width; marks live in the shared DSS rule.
No appearance constants or manual hit tests are required in application code.
Long labels clip horizontally; rows have uniform height. Resize clamps scroll
and reveals current using the same controller as lists.

## Tables and rename

`View::table(key, label, model, state, selection_mode, on_change)` uses the same
virtual row selection, viewport, reveal and two-axis scrollbar controller as lists
and trees. `TableModel` owns immutable keyed columns and rows; duplicate or empty
keys, mismatched cell counts and non-finite/non-positive explicit widths fail
construction. Widths omitted by the caller use DSS `table-column-width`.

`TableValue::{Text,Integer,Unsigned}` sorts signed/unsigned numbers numerically
without overflow and text lexically, retaining source order for equal values.
Numbers precede text. Sorting never changes row identity. Header click toggles
ascending/descending order; F4 toggles the current column, Left/Right selects a
column. Existing list selection keys and Shift/Ctrl rules apply to rows. The
model can reject every proposed change. Row/column identity is captured before
committing a focused spinbox, then resolved again after the resulting view update.
A removed, disabled or out-of-modal-scope target cannot receive the stale click.

Only visible rows produce paint payloads. Column text outside the horizontal
viewport is culled. The header stays above the scrolled rows. `table_cell` exposes
role, label, row/column indices and selected/current/disabled/editable metadata.

`.renaming(state, on_event)` enables F2 on list/tree/table. The callback receives
`Begin`, `Change`, `Commit`, `Cancel`; the model owns the draft and its acceptance.
Tables require an editable column. A real shared Input is laid out over the final
scrolled cell/row, clipped to the collection viewport. Opening selects its text;
Enter/Tab commits, Escape cancels, closing restores the collection focus. Rejected
edits preserve editor history/selection. No application painting or hit test is
needed. There can be one active collection editor per view.

Verification: `tests/table.rs` covers numeric extremes/stability, disabled rows,
selection/sort rejection, pointer identity after model rebuild, virtual 10,000-row
scrolling, modal scope, rename lifecycle/history and resize/pixels in both themes.
`tools/sdk_collections_markers.json` drives actual F4/F2/edit/Enter/End/resize in
QEMU and captures the editor and table.

## File dialogs

`FileDialog<P: DirectorySource>` supplies Open, Save and Directory selection over
shared file-browser entries/path normalization. All labels and filters belong to
`FileDialogOptions`; the component does not perform file writes. The native
`NativeDirectorySource` reads the filesystem through existing ABI calls. `gui_files`
uses the shared browser path helper, SDK menus and filename dialogs over its
icon canvas; see the [GUI migration receipt](desktop-transfers.md#gui-application-boundaries).

Embed `picker.view().map_messages(ParentMessage::Picker)` as the dialog argument of
`View::modal`, forwarding child messages through the parent model. It then shares
the real parent's layout, modal focus, popup/keyboard scope and opener restoration.
`dunit::ui::choose_file(options)` is a convenience standalone window returning an
optional path. It does not establish compositor modality over another window.

The dialog includes navigation history, Up, location entry (Enter navigates),
refresh, hidden-file toggle, extension filters, directory-first initial listing,
controlled sortable table, filename entry, errors and cancel/default actions.
Directories stay visible through extension filters; matching is ASCII case
insensitive. Device entries cannot be selected. Invalid/duplicate provider names
reject the complete candidate listing. Failed navigation preserves the previous
listing and filename draft. Relative locations normalize against the current
folder; supplied initial directory must be absolute.

Acceptance revalidates actual target metadata. Open requires a regular file;
Directory requires a directory; Save requires an existing parent directory and
valid single-segment filename. Existing files open a separate confirmation view.
Confirmation is bound to the original path and revalidates it again. Returning a
path is no filesystem transaction or overwrite guarantee: callers must handle
errors/races when they subsequently open/write it.

Native `fs::read_dir` grows its snapshot buffer to the existing ABI's 1024-entry
limit. A full maximum buffer returns `ENOBUFS` rather than silently presenting a
possibly truncated directory. Native metadata errors are reported by the dialog;
optional per-file size does not prevent displaying an otherwise valid entry.

Verification: host provider tests cover error rollback, stale entries, filters,
history, overwrite identity, all three modes and modal message composition. The
combined QEMU fixture lists real MemFS files, confirms/declines overwrite,
cancels the picker and verifies the existing file bytes remain unchanged.

## Verification

Host paths below are relative to `runtime/dunit-component/tests/` unless stated
otherwise. Native fixture names mean
`python3 tools/test_sdk_gui_fixture.py NAME --accel tcg`, which invokes the sole
OS harness, `tools/qemu_test.py`. Existing marker contracts stay authoritative;
the matrix does not replace their assertions with checklist entries.

| Family | Behavior and state coverage | Host contract | Native contract |
| --- | --- | --- | --- |
| Text and containers | Current role/name, passive focus behavior, message-driven content, retained pixels | `components.rs`, `layout_style.rs` | `theme` |
| Push buttons | Matching pointer press/release, cancellation, Tab, held Space/Enter, one activation on break | `buttons.rs`, `input_lifecycle.rs` | `buttons`, `lifecycle` |
| Toggle, checkbox, radio, switch | Model-owned checked/pressed metadata, keyboard/pointer activation, radio roving and disabled skip | `checkable.rs`, `input_lifecycle.rs` | `checks`, `lifecycle` |
| Input | Grapheme-safe editing/selection/history, placeholder/password/read-only/limits, model rejection, horizontal viewport | `input.rs`, `click_selection.rs`, `input_lifecycle.rs` | `input`, `collections`; native modifiers |
| Textarea | LF editing, vertical navigation, visual soft wrap/affinity, selection and shared scrollbars | `multiline.rs`, `soft_wrap.rs`, `input_scroll.rs` | `multiline`, `collections`, `scroll_xy` |
| Text gestures | Single/double/triple clicks, word/line drag, Shift/Ctrl anchors, stationary edge autoscroll | `click_selection.rs`, `input_lifecycle.rs`, `input_scroll.rs` | `collections`, `autoscroll`; native modifiers |
| Scroll areas | Both axes, nested wheel bubbling, focus reveal, track/thumb capture, clamp, resize, timed auto-hide | `scroll.rs`, `scroll_xy.rs`, `scroll_visibility.rs` | `scroll`, `scroll_xy`, `autoscroll` |
| Sliders | Horizontal/vertical geometry, integer bounds/steps, keyboard/track/drag, controlled rejection and cancellation | `sliders.rs`, `vertical_sliders.rs` | `slider`, `vertical_slider` |
| Spinbox | Numeric draft, commit/cancel, bounds/steps, buttons, keyboard, read-only/disabled and rejected edits | `spinboxes.rs`, `focus_order.rs` | `spinbox` |
| Progress and busy | Range metadata, passive input, timed progress pixels, pause/disabled/reduced-motion/clipping limits | `progress.rs` | `progress` |
| Select and menus | Controlled select, nested action menus/separators, menu-bar roving/mnemonics, context trigger, pointer/keyboard capture | `menus.rs`, `input_lifecycle.rs` | `menus`, `lifecycle` |
| Tooltips | Delayed appearance, clipping/wrapping, noninteractive metadata, input/focus/rebuild cancellation | `menus.rs`, `input_lifecycle.rs` | `menus`, `lifecycle` |
| Tabs | Controlled selected page, disabled skip, roving headers/Ctrl+Tab, only active page mounted | `workspace.rs` | `workspace` |
| Splitter | Both axes, shared paint/hit geometry, keyboard/drag, pane minima, resize and controlled rejection | `workspace.rs` | `workspace` |
| Toolbar and status | Single roving toolbar stop, Home/End, stable remembered focus; passive named status | `workspace.rs`, `focus_order.rs` | `workspace`, `collections` |
| Dialog/modal/message box | Client-local focus scope, default/cancel, opener restoration, disabled background, bounded layout | `workspace.rs`, `focus_order.rs` | `workspace` |
| List | Single/multiple/range selection, modifiers, stable row keys, empty/disabled/model rejection, virtual rows | `list.rs` | `list`; native modifiers |
| Tree | Validated hierarchy, expansion/collapse/promotion, visible-row selection and tree metadata | `tree.rs` | `tree`; native modifiers |
| Table, sort and rename | Stable typed sorting, columns/cell metadata, virtual paint, F2 draft/commit/cancel, stale-target cancellation | `table.rs` | `collections`; native modifiers |
| File dialog | Open/Save/Directory, native listing, navigation/filter/history, actual metadata, overwrite/cancel without writes | `file_dialog.rs` | `collections` |
| Shared focus and routing | Stable Tab order, scoped mnemonics, capture/bubble once, prevent/default, revalidation after callbacks | `focus_order.rs`, `routing.rs`, `input_lifecycle.rs` | `collections`, `workspace`, `lifecycle` |
| DSS layout and appearance | State-aware flex/grid constraints, clipping, resize, rollback, light/dark retained-vs-fresh pixels | `layout_style.rs`, `themes.rs`, `effects.rs`, `backdrop.rs` | `theme` |

The list/tree/table host workloads use 10,000 entries and assert viewport-bounded
materialized rows. The native `collections` showcase asserts fewer than 30 visible
rows in its 10,000-row table and emits `[WIDGETS] ok` only after its routing,
editing, sorting, picker, gesture and resize assertions have completed.

The 2 November 2026 acceptance record contains 650 host tests, 19 Python harness
checks and 20 native TCG contracts with 433 required-marker checks.
[m7-6-results.json](m7-6-results.json) retains the machine-readable case results.
These are historical results, not a fresh verification of a later source state.

Native fixture names use `python3 tools/test_sdk_gui_fixture.py NAME --accel tcg`.
The wrapper invokes `tools/qemu_test.py`; serialize all guest builds and boots.
State, exact pixel-reference and event-count assertions determine acceptance.
Screenshots supplement those assertions. Theme goldens compare dark, light and
restored full client pixels. No assertion or forbidden marker may be relaxed to
make a run pass. The collections workload also requires the DF entry regression
documented in [kernel-entry-flags.md](../architecture/kernel-entry-flags.md).

Client-local popups/modality, Unicode editing and role/name metadata do not
provide cross-surface popup roles, global grabs, system IME or a screen reader.
Those need separate system/protocol contracts. Application migration and wider
release/performance gates remain tracked in the project roadmap.
