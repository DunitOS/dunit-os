# DWM shell configuration (schema 2)

The configuration remains a checked single-line TOML subset. Strings use double
quotes and arrays stay on one line. Settings saves preserve shell, binding and
rule tables. A failed validation rejects the complete candidate configuration.
Full-desktop imports and source-editor saves require a complete application
registry; theme-only changes use the separate checked overlay path.
Schema 1 remains accepted; without explicit panel tables the legacy panel, dock
and tray settings resolve to a shell tree without rewriting the source file.

## Panels and components

Each `[panels.ID]` declares an independent surface. `children` contains component
IDs in display order. Each `[components.ID]` declares one instance; containers
can nest children. Every component must have exactly one parent and be reachable
from a panel. IDs use ASCII letters, digits, `_` and `-`, up to 64 bytes. Resource
ceilings are 64 panels, 512 components and 16 nesting levels.

```toml
[meta]
schema_version = 2

[shell]
stylesheet = "/assets/dwm/shell.dss"

[panels.bottom]
edge = "bottom"
align = "center"
size = 56
length = 0
margin = 8
padding = 6
gap = 8
reserve = true
autohide = false
class = "dock"
children = ["apps", "tools", "clock"]

[components.apps]
kind = "apps"
size = 40
entries = ["gui_terminal", "gui_files"]
source = "both"

[components.tools]
kind = "app_group"
label = "Tools"
icon = "/assets/icons/aqua/mime_folder.svg"
entries = ["gui_calc", "gui_settings"]
presentation = "grid"

[components.clock]
kind = "clock"
format = "%H:%M"
font_px = 18
interval_ms = 1000
```

Application references must match the document's `[application.ID]` registry.
Explicit trees replace the legacy strip composition. Panel `edge` is `top`,
`bottom`, `left` or `right`; `align` is `start`, `center` or `end`. `size` is the
cross-axis thickness, `length` is the axis length (zero fits content). Layout
clamps surfaces to the display. Margin, padding and gap use pixels. `reserve`
controls whether windows leave room for the surface. `enabled` defaults true.
The optional `class` selects DSS styling; stylesheet paths must be absolute
without parent traversal.

Component kinds: `launcher`, `quicksettings`, `workspaces`, `apps`, `app_group`, `clock`,
`analog_clock`, `tray`, `title`, `system_monitor`, `button`, `indicator`, `spacer`, `container`.
Components accept `enabled`, `class`, `align`, `size`, `width`, `height`, `grow`
and `gap`. Zero width/height are intrinsic. Containers accept `children` and
`direction = "horizontal"` or `"vertical"`. Application components accept
`entries`; groups accept `presentation = "grid"`, `"list"` or `"fan"`.
Text controls use `label`, `font_px` (zero inherits), optional `color` and
`format`. Refresh `interval_ms` ranges from 16 to 3600000. Button `action` uses
the same checked vocabulary as keyboard bindings below.

## Key bindings

```toml
[bindings.terminal]
scancode = 20
modifiers = "super+shift"
action = "launch:gui_terminal"

[bindings.second-workspace]
scancode = 3
modifiers = "super"
action = "workspace:2"
```

Scancodes are keyboard make codes (1–127). Modifier names `shift`, `ctrl`, `alt`
and `super` combine with `+`; empty or `none` means no modifiers. Duplicate
chords and repeated modifiers reject the document. Up to 128 bindings are allowed.
Actions are `launcher`, `close`, `maximize`, `minimize`, `retile`,
`launch:APPLICATION_ID` and `workspace:NUMBER`. Workspace numbers are one-based
and cannot exceed `[workspaces] count`. Commands are typed desktop operations;
arbitrary shell commands are not accepted.

## Window rules

```toml
[rules.defaults]
app = "*"
width = 800
height = 600

[rules.terminal]
app = "gui_terminal"
workspace = 2
floating = true
opacity = 240
animation = "genie"
```

Rules are applied in document order. Each matching rule overrides only its
supplied fields, so the example retains the general width and height for the
terminal. The selector is a registry ID or `*`. Optional fields are workspace,
`x`, `y`, `width`, `height`, `floating`, `opacity` and `animation`. Coordinates
may be negative (range -16384 to 16384); dimensions range from 1 to 16384;
opacity ranges from 0 to 255. Up to 128 rules are allowed. Rules set initial
window policy; interactive changes and Lua layout are handled by the compositor.

## Lua events

`[scripts] events = "/assets/dwm/scripts/events.lua"` selects the event script
separately from the existing `layout` script. The empty string disables event
scripts. Runner/layout/event paths are absolute and reject parent traversal.
The native compositor executes animations, while the isolated Lua runner
produces bounded policy data outside the frame rendering loop.

## Desktop clock widget

The independent desktop widget file `/system/share/dwm/widgets/clock.toml`
accepts `style = "analog"` or `"digital"`, `size` (24–1024 pixels), `font_px`
(6–128) and `interval_ms` (16–3600000), in addition to `enabled`, `accent` and
`corner`. Defaults are digital, size 160, font 18 and 1000 ms. Widget candidates
apply atomically: invalid dimensions or unknown style keep the previous values.

Applications components accept `source = "pinned"`, `"running"` or `"both"`.
The default is pinned; both appends running applications without duplicates.
Topology changes rebuild layout from cached styles and retain the glyph atlas.
Component `icon` accepts an absolute SVG or raw RGBA path without traversal.
Images are loaded at configuration boundaries and share a 16 MiB scene budget.

## User and system documents

The system document remains `/system/share/dwm/default.toml`. If present,
`/persist/config/dwm.toml` is a complete user profile and takes precedence.
Settings edits preserve the active document; theme overlays merge into it.
The Desktop profiles page activates a checked user profile. `setup import-user
/absolute/profile.toml` provides the same operation in the terminal; `setup
import-desktop` retains the existing active-document import workflow. Persistence
across reboot depends on the mounted `/persist` filesystem; a Live ISO uses RAM.
A malformed user profile is rejected on reload; initial startup can recover the
packaged system desktop. Lua reload candidates are evaluated in separate
bounded processes before the current desktop state is replaced.

## Policy precedence and limits

Rules establish initial geometry, workspace, opacity and floating state. Matching
rules merge in declaration order. Manual dragging releases a window from Lua
tiling; the `retile` action returns it to the layout script. Maximized windows
remain outside the tiling set. Explicit keybindings take precedence over legacy
shortcut defaults. Lua animation geometry commits through the unchanged gui-v1
configure/acknowledge path; manual geometry interaction cancels those tracks.

The panel scene is bounded to 64 surfaces, 512 components and depth 16. This is
a safety ceiling, not a fixed desktop arrangement. Independent unreserved,
content-sized panels can contain clocks, monitors, indicators and buttons.
The clock currently uses the existing monotonic uptime source rather than RTC
calendar time. System monitor data comes from the existing system-information
API. Genie owns a frozen client snapshot during the transition, so the client
may continue updating without invalidating sampled pixels. Rendering remains
software based; large blur areas and many surfaces cost CPU and memory.

Bundled event policies live in `scripts/events/`; the layout picker discovers
nonempty `.lua` files in the configured layout's parent directory. Empty drafts
are not executable layout choices. Both layout and event paths remain editable.
