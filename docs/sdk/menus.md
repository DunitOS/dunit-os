# Select, menus, menu bars, context menus and delayed tooltips (M7.6)

All five controls share a client-side overlay layer. Application owns transient
popup/tooltip state; the model owns choices and action effects. ComponentWindow
routes the existing seat events and wait-set deadlines. No syscall or gui-v1
format change is needed.

## Declarative API

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

## Selection, navigation and capture

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

## Placement and painting

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

## Tooltips and accessibility

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

## Scope and verification

This increment covers noneditable select, action menus with nested submenus and
separators, menu bars, context providers and plain delayed tooltips. Popups are
inside one client surface. Current gui-v1 has no keyboard-focus-out/global
outside-click notification: a click inside the client dismisses a popup, but a
click in another window cannot automatically dismiss it through this API.
Cross-surface popup windows and that focus contract remain separate work.

Fifteen host scenarios cover validation/accessibility, controlled rejection,
roving focus, mnemonics, nested/empty/disabled menus, pointer cancellation and
outside-click capture, context focus, rebuild invalidation, clipped/long menus,
dark/light pixel/damage equivalence, resize, placement and tooltip deadlines,
lifecycle and held-button suppression. Existing SDK suites run as regressions.

```sh
python3 tools/test_sdk_gui_fixture.py menus --accel tcg
```

One native showcase verifies all five controls through real keyboard/pointer
input, no click-through, timer timestamps, action/model outcomes and resize of
an open submenu. `tools/sdk_menus_markers.json` requires the behavioral markers,
forbids panics and captures separate select/tooltip/context/submenu screenshots.
