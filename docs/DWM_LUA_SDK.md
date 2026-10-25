# DWM Lua SDK 2

`[scripts]` keeps `layout` for existing one-shot tiling scripts. Set `events` to an
absolute Lua script path to enable event subscriptions while `enabled = true`.
The shipped `/assets/dwm/scripts/events/macos.lua` selects Genie for minimize and
restore, and scale for opening windows. The host resolves the animation target
from the actual panel geometry.

```lua
local dwm = require('dwm')
local animation = require('animation')
assert(dwm.api_version == 2)

animation.register('gentle', function(t)
    return 1 - (1 - t)^3
end)

dwm.on('window::created', function(win)
    dwm.animate(win, {
        property = 'opacity', from = 0, to = 1,
        duration = 250, easing = 'gentle',
    })
end)

dwm.on('window::minimized', function(win)
    dwm.animate {
        window = win.id, target = win.target,
        effect = 'genie', duration_ms = 420, easing = 'ease_out',
    }
end)
```

`animation` is also available as `dwm.animation` and as a global. Registered
curves and function-valued `easing` are sampled into 33 signed Q16 values before
returning to the compositor. They must have endpoints 0 and 1; intermediate
samples may overshoot within [-1, 2]. Rendering never calls Lua. Built-in curve
names are `linear` and `ease_out`; registered curves can replace names locally.
The native `spring` effect has its own bounded spring behavior.

Event subscription names and equivalent host names:

| SDK name | Host name |
| --- | --- |
| `window::created` | `window_open` |
| `window::closed` | `window_close` |
| `window::focused` | `window_focus` |
| `window::minimized` | `window_minimize` |
| `window::restored` | `window_restore` |
| `window::maximized` | `window_maximize` |
| `workspace::changed` | `workspace_change` |
| `panel::clicked` | `panel_click` |
| `config::reloaded` | `reload` |
| `panel::hover` | `panel_hover` |
| `launcher::opened` | `launcher_open` |
| `launcher::closed` | `launcher_close` |
| `widget::updated` | `widget_update` |
| `widget::clicked` | `widget_click` |
| `theme::changed` | `theme_change` |

The callback receives `id`, `window.id`, `name` (the host name), and `target`.
Window lifecycle events also identify `app`; component events expose the stable
configured `component` ID, so scripts can select widgets without hardcoding tokens.
Target IDs are native host tokens; a script cannot acquire a surface pointer or
process handle. The host validates the window still exists before applying a
returned descriptor. Scalar `from`/`to` must be finite and satisfy the property range:

| Property | Values | Native wire units |
| --- | --- | --- |
| `opacity` | 0..1 | Q16 (65535 = 1) |
| `scale` | 0..16 | Q16 (65535 = 1) |
| `x`, `y`, `translation_x`, `translation_y` | -16384..16384 | Integer pixels |
| `width`, `height` | 1..16384 | Integer pixels |
| `blur`, `shadow`, `corner_radius` | 0..1024 | Integer pixels |
| `color` | 0..4294967295 | Integer packed color |

Pixel and color values must be integral; opacity and scale accept fractions.
The host interpolates color channels independently. Descriptive effects are `fade`, `slide`, `scale`, `spring`,
and `genie`; the native host supplies their window geometry and direction.
Durations are integer milliseconds in [0,2000]. Unknown fields are rejected.

## Desktop commands

`dwm.command('launch:terminal')` submits a descriptor to the native host's same
checked action dispatcher used by panel buttons and keybindings. The allowlist
is `launcher`, `close`, `maximize`, `minimize`, `retile`, `workspace:N`, and
`launch:APP_ID`. The host resolves configured application IDs and workspace
counts before executing the batch. Arbitrary executables, shell strings,
filesystem operations, and kernel handles are not exposed. Up to 32 command
descriptors follow the animation records as `COMMAND <token>` before `END`.
A malformed command rejects the entire response, including earlier commands.
Commands execute after the complete batch has passed validation.

## Lua widgets

An Indicator component can display text supplied by a Lua event handler:

```lua
dwm.on('widget::updated', function(event)
    if event.component == 'activity' then
        dwm.widget.set(event.target, {text = 'Ready', color = 0xffaabbcc})
    end
end)
```

The target token identifies a component (index + 1); the native shell validates
its current type before committing updates. Each batch can update up to sixteen
widgets. Text must contain at most 256 UTF8 bytes and no control characters; an empty string
clears the text. The optional packed ARGB integer `color` overrides text color;
omitting it retains the component's current color. The wire form is
`WIDGET <token> <decimal-color-or--1> <hex-encoded-UTF8-or-dash>`, avoiding injection
through spaces or newlines. All records are validated before the shell updates
its text atlas or applies any part of the batch.

## Isolation and failure

Event policies run in a long-lived isolated worker. The script is compiled and
subscriptions registered once; variables, registered curves and subscriptions
persist across events. Settings reload cancels the worker and starts fresh state
on the next event. A malformed batch, process exit or timeout also discards the
worker; queued later events start a replacement. Timer callbacks are not exposed.
The sandbox has pure libraries and the two explicit modules; no `io`, `os`,
package loading, filesystem access, spawning, or per-frame callbacks are exposed.

The evaluator limits source to 64 KiB, Lua allocations to 1 MiB and execution to
100,000 Lua instructions. Each event may emit at most 32 actions and register at
most 64 handlers for each event and 32 named curves. The host retains a bounded
32-event queue. Widget timer and hover updates coalesce by target. Lifecycle
events precede these updates and evict an update on queue overflow, preserving
minimize/restore ordering. Overflow without an evictable update is reported.

The existing `DWM_LUA 1` layout response is unchanged. Event mode is selected by
`DWM_LUA_EVENT`, `DWM_LUA_WINDOW`, and `DWM_LUA_TARGET` environment variables in
addition to the existing script and work-area variables. Desktop event workers
instead set `DWM_LUA_DAEMON=1` and send bounded `EVENT <name> <window> <target>`
lines on the controlled PTY stdin; `STOP` requests a clean shutdown. The Lua
instruction quota resets for every dispatched event. Both event modes return
`DWM_EVENT 2 <name> <window> <count>`, validated `ANIMATE` records and `END`.
Each `ANIMATE` contains window, target, effect, duration, property (`none` or
a supported scalar name), signed 64-bit scalar endpoints, and exactly 33 signed Q16 curve samples.
Layout and one-shot event consumers wait for successful exit; the persistent
worker host accepts only a complete bounded event batch before exposing actions. Malformed output, invalid samples, quota exhaustion and failed
processes reject the whole batch. Timeouts terminate/reap the worker; rendering
continues using the native fallback policy.

## Verification

`tools/test_dwm_host.sh` tests malformed and complete native event/layout parser
responses. The terminal `tools/lua_markers.json` suite runs `dwm_lua_test` against
real isolated children, including event subscriptions, custom curve sampling,
the scalar property API, state retained across two requests to one worker, and
atomic rejection of NaN easing values.

## Component motion and curves

For a component event, submit `dwm.animate {window=0, target=event.target,
property="opacity", from=0.6, to=1, duration=180, easing="ease_out"}`.
The target token is valid for the current scene until configuration reload.
Component geometry, opacity, text color, radius, shadow and blur are sampled
natively; parent translation/scale and opacity propagate through nested trees.
Window geometry commits through the existing configure protocol on completion.
Damped custom curves may approach endpoints within one percent; compilation
pins those residuals to exact endpoints, while rejecting larger mismatches.
Persistent workers report a bounded subscription mask so unsubscribed periodic
widget events do not execute Lua or produce protocol traffic.

## Scene layers and panels

`dwm.targets.launcher`, `dwm.targets.workspace`, and `dwm.targets.notifications`
select native popup, window-workspace, and notification layers. Use `window=0`
and a scalar property with these targets. Workspace motion transforms window
input coordinates along with rendering; desktop panels retain their own geometry.
Launcher close transitions retain their native content until completion.
Notification opacity, geometry, text color, radius, blur and shadow apply per card.

Component callbacks also supply `event.panel`, the containing panel's native
token. Animating that token transforms the panel and its complete child tree.
Panel `color` controls its background; component `color` controls text color.
Target tokens expire on scene reload; use the current callback tokens or the
named `dwm.targets` entries rather than retaining component indices.

Persistent EVENT lines may append a stable app/component identifier and a panel
token. The host sends these from the committed scene registry. Widgets accept
only component targets; scene-layer and panel tokens cannot update widget text.
