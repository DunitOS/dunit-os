# Desktop configuration and Lua policy

The userspace gui_server owns windows, focus, capabilities, protocol sequencing,
shell surfaces and native animation. Schema-1 and schema-2 configuration resolve
into one typed component tree. Layout Lua computes one-shot placement snapshots;
event Lua runs in a separate persistent worker and returns validated descriptors.
Rendering never invokes Lua. These mechanisms retain the kernel ABI and gui-v1.

Complete examples are `assets/dwm/frost-desktop.toml`,
`assets/dwm/multi-panel-desktop.toml`, `assets/dwm/styles/frost.dss` and
`assets/dwm/scripts/events/macos.lua`. Frost uses a full-width bottom surface;
the multi-panel example adds an unreserved auto-hidden utility panel. Fixed strip
drawing and bitmap 3×5 shell text were replaced by configured surfaces and text.
The shell stays inside gui_server; [performance acceptance](../performance/gui-performance.md#retained-shell)
does not accept external-shell replacement or the <=16 ms goal.

## Configuration

Configuration uses the checked single-line TOML subset: double-quoted strings,
one-line arrays. A complete candidate is read, parsed, validated and prepared
before replacement. Invalid/unknown/clamped theme values reject the entire
overlay; invalid desktop fields reject the entire document. Scene/font preparation
and Lua preflight occur before commit. Rejection retains the last good live state.

The system document is `/system/share/dwm/default.toml`. A present
`/persist/config/dwm.toml` is a complete user profile and takes precedence.
Malformed user profiles fail reload; initial startup can recover the packaged
system desktop. Reboot persistence requires mounted persistent `/persist`;
a Live ISO may use RAM. Ordinary theme/layout settings remain session RAM.

Settings source editors preserve the active document, shell/binding/rule tables
and independent diagnostics. Full-desktop imports/source-editor saves require a
complete `[application.ID]` registry. Theme overlays merge through a separate
validator. The Desktop profiles page activates a checked user profile;
`setup import-user /absolute/profile.toml` does the same. `setup import-desktop`
retains the active-document workflow. These are source editors, not a graphical
panel designer. Global font paths and component font sizes are configurable.

Schema 1 remains accepted. Without explicit panels, legacy panel/dock/tray
settings resolve into the tree without rewriting the source file.

## Panels and components

Each `[panels.ID]` declares one surface with ordered child IDs. Components may
nest through containers. Every component must have exactly one parent and be
reachable from a panel. IDs contain ASCII letters/digits/underscore/hyphen and
are at most 64 bytes. Ceilings are 64 panels, 512 components and depth 16.

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

Application entries refer to this document's registry. Explicit trees replace
legacy strips. Panel edge is top/bottom/left/right; align is start/center/end.
Size is cross-axis thickness; length is axis length (zero fits content). Margin,
padding and gap are pixels. Geometry clamps to display bounds and is shared by
rendering, hit testing and work-area reservation. Reserve controls window space;
enabled defaults true. Class selects DSS styling. Stylesheet/icon paths are
absolute without parent traversal. Configured scenes share a 16 MiB image budget;
images load at configuration boundaries. Topology changes reuse cached styles
and the glyph atlas.

Kinds are launcher, quicksettings, workspaces, apps, app_group, clock,
analog_clock, tray, title, system_monitor, button, indicator, spacer and container.
Common fields are enabled, class, align, size, width, height, grow and gap. Zero
width/height is intrinsic. Containers take children and horizontal/vertical
direction. Apps/groups take entries; source is pinned (default), running or both,
with running entries deduplicated. Group presentation is grid/list/fan. Text uses
label, format, optional color and font_px (zero inherits). Refresh interval_ms
is 16..3600000; button action shares the checked binding vocabulary.

The separate `/system/share/dwm/widgets/clock.toml` accepts analog/digital style,
size 24..1024, font_px 6..128, interval_ms 16..3600000, enabled/accent/corner.
Defaults are digital, size 160, font 18, interval 1000 ms. Invalid candidates
retain prior values. Clock displays monotonic uptime; RTC/calendar/time-zone
support is a separate feature. Monitor data uses the existing system-information API.

## Bindings and window rules

```toml
[bindings.terminal]
scancode = 20
modifiers = "super+shift"
action = "launch:gui_terminal"

[bindings.second-workspace]
scancode = 3
modifiers = "super"
action = "workspace:2"

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

Make scancodes are 1..127. Shift/ctrl/alt/super combine with `+`; empty/none means
no modifiers. Duplicate chords/repeated modifiers reject the document. At most
128 bindings are allowed; explicit bindings override legacy defaults. Actions
are launcher, close, maximize, minimize, retile, launch:APPLICATION_ID and
workspace:NUMBER, one-based within configured workspace count. Arbitrary shell
commands are rejected.

Up to 128 rules merge in declaration order, overriding only supplied fields.
Selector is registry ID or `*`. Fields are workspace, x/y (-16384..16384),
width/height (1..16384), floating, opacity (0..255) and animation. Rules establish
initial policy. Manual drag releases Lua tiling; retile returns it to the script.
Maximized/minimized windows do not participate in layout. Floating restore
geometry/detachment survive display-session recreation; disabling Lua restores
remembered floating geometry.

## Themes and assets

Production defaults to Mojave Dark: original vector dune wallpaper, dark
surfaces, top panel, centered bottom dock, left circular controls and centered
titles. Unavailable services are not drawn as fake menu entries. Settings Themes
discovers presets from `[desktop] themes`: Mojave Dark, macOS Light, Nord and
Green Tea. Terminal import uses the same complete-overlay validation:

```text
exec setup import /assets/dwm/themes/mojave-dark.toml
```

Reopened apps inherit desktop colors; explicit `apps/*.toml` colors override
inheritance. Terminal ANSI palette remains independently configured. Layout owns
dock_edge, dock_centered, dock_w, dock_padding, dock_gap, dock_margin,
dock_tile_alpha, traffic_lights and show_workspaces; theme owns close/minimize/
maximize colors. Green Tea restores the left dock and legacy controls.

`runtime/dunit-svg` is the no-std static renderer. `userspace/dunit_image` supplies
one bounded VFS loader for compositor and Files. Registry icon stems try
`/assets/icons/<desktop.icon_theme>/<stem>.svg`, then 32×32 RGBA only if SVG is
absent. Absolute icon/logo paths may select SVG. A present invalid/unsupported
SVG reports an error without silently substituting a bitmap; missing assets
retain initials/primitive fallback.

Compositor SVG rasterization uses dock size plus hover margin and caches actual
dimensions/pixels for launcher/task switcher/panel/dock. Reload rebuilds caches.
Files uses the same loader at its configured icon size. SVG wallpapers render
once at active framebuffer size on startup/mode change/reload; transparent pixels
blend with configured background. Bitmap wallpapers/Breeze RGBA remain supported.
Default/Light/Nord select 13 original Aqua SVGs and their MIT license in
`assets/icons/aqua`; Green Tea selects Breeze. Wallpaper source is
`assets/wallpapers/mojave.svg`. Icon theme, app icon, logo and wallpaper are config.

The [SVG runtime reference](../../runtime/dunit-svg/README.md) owns supported
elements and detailed limits: 1 MiB XML, 4096 nodes/draw visits, depth 32,
16384 segments, 128 stylesheet rules/64 KiB CSS, 256 gradient stops, 12 MiPixels
output and 24 MiPixels opacity layers. It rejects unsupported browser features,
cycles and invalid geometry without a partial image. No parser/policy enters kernel.

## Display modes

Settings offers keep-boot-mode and modes through 2560×1440, 2560×1600, 2880×1800
and 3072×1920. Existing `[display]` width/height and compositor session rebuild
apply changes. Captured client PID and pointer button edges survive recreation:
holding the button does not create repeat clicks, and release reaches the client.

Bochs/DISPI measures the matching PCI VGA BAR0, intersects it with reported video
memory, and maps capacity before userspace startup. Validation checks dimensions
and mapped memory, falling back to boot mapping capacity without a suitable BAR.
Fixed Limine/GOP behavior remains. Framebuffer aliases preserve boot PAT/cache
selection, translating huge-page PAT to 4 KiB leaves; register MMIO is uncached.
Treating framebuffer VRAM as uncached registers would slow full-frame copies.

## Lua layout

`userspace/system_apps/dwm_lua/main.c` embeds official vendored Lua 5.5.1 as a
static application. Runtime/vendoring details are in [lua-runtime.md](lua-runtime.md).
Layout configuration and invocation are:

```toml
[scripts]
enabled = false
runner = "/app/dwm_lua"
layout = "/assets/dwm/scripts/columns.lua"
events = ""
timeout_ms = 500
```

```text
dwm_lua /assets/dwm/scripts/columns.lua 1200 800 3
```

Environment invocation uses DWM_LUA_SCRIPT/WIDTH/HEIGHT/WINDOWS through existing
pty_spawn_env, without adding argv transport. Paths are absolute without parent
traversal. Width/height are usable work area (excluding reserved margins),
1..16384; count is 0..64 and still respects configured max_windows. The ordered
target list belongs to the compositor; Lua gets no PID or handle. Source is text
only, up to 64 KiB, loaded in 4096-byte chunks plus one overflow byte to respect
MAX_USER_COPY. Missing/invalid invocation or source fails status 2.

Lua sees workarea={width,height,count} and returns exactly:

```lua
return {
    rectangles = {{x=0, y=0, w=400, h=800},
                  {x=400, y=0, w=400, h=800},
                  {x=800, y=0, w=400, h=800}},
    animation = {duration_ms=140, easing='ease_out'},
}
```

Rectangles use contiguous integer indices 1..count, exact record fields, integer
nonnegative x/y, positive w/h and complete work-area containment. Overlap is legal.
Zero windows returns an empty table. Duration is 0..2000 ms and easing linear or
ease_out. Columns/monocle are complete examples. Output is decimal ASCII:

```text
DWM_LUA 1 1200 800 3
RECT 1 0 0 400 800
RECT 2 400 0 400 800
RECT 3 800 0 400 800
ANIMATION 140 ease_out
END
```

Zero count omits RECT lines. Validation precedes first output; errors are
`dwm_lua:` stderr diagnostics. Status 0 is complete/flushed output, 1 evaluation/
validation/allocation/output failure, 2 invocation/read failure. Transport can
leave a partial result or merge diagnostic output; parent tolerates CRLF but
requires END, exit 0, matching request header, exact fields/indices and no extra
records. It buffers at most 16 KiB, drains at most four 1 KiB chunks per frame,
then applies once. It kills/reaps timeout/failure after configured 50..5000 ms.

Workspace/window/work-area/config changes invalidate snapshots and cancel stale
results. Every cell must fit decorations before commit. Resize uses existing
CONFIGURE/ACK/import/commit; `Lua geometry committed` requires actual matching
client buffers. Failures preserve geometry. Layout runs on snapshot changes,
not each frame. Drag cancels tracks/detaches placement until retile or reload.

## Lua events

Nonempty `[scripts] events` enables subscriptions when scripts.enabled is true.
The worker compiles/registers once; variables, curves and handlers persist across
events. Reload discards it and the next event starts fresh state. Process exit,
malformed output or timeout also discards it; later queued events start replacement.

```lua
local dwm = require('dwm')
local animation = require('animation')
assert(dwm.api_version == 2)
animation.register('gentle', function(t) return 1-(1-t)^3 end)
dwm.on('window::created', function(win)
    dwm.animate(win, {property='opacity', from=0, to=1,
                     duration=250, easing='gentle'})
end)
dwm.on('window::minimized', function(win)
    dwm.animate {window=win.id, target=win.target, effect='genie',
                 duration_ms=420, easing='ease_out'}
end)
```

Animation is also dwm.animation and a global. Built-ins are linear/ease_out;
registered names can locally replace them. Curves/function easing become exactly
33 signed Q16 samples with endpoints 0/1 and overshoot within [-1,2]. Damped
endpoint residuals within one percent pin exactly; larger mismatches fail. Lua
never runs during rendering. Native spring has its own bounded behavior.

| SDK subscription | Native name |
| --- | --- |
| window::created / closed / focused | window_open / window_close / window_focus |
| window::minimized / restored / maximized | window_minimize / window_restore / window_maximize |
| workspace::changed | workspace_change |
| panel::clicked / hover | panel_click / panel_hover |
| config::reloaded | reload |
| launcher::opened / closed | launcher_open / launcher_close |
| widget::updated / clicked | widget_update / widget_click |
| theme::changed | theme_change |

Callbacks receive id, window.id, native name and target; window events identify
app, component events expose stable configured component ID and containing panel
token. Host revalidates current window/scene identity before applying descriptors.
No pointer or process handle is supplied. Durations are integer 0..2000 ms;
unknown fields/NaN/nonfinite endpoints fail.

| Scalar property | Range | Transport units |
| --- | --- | --- |
| opacity | 0..1 | Q16, 65535=1 |
| scale | 0..16 | Q16, 65535=1 |
| x/y/translation_x/translation_y | -16384..16384 | Integer pixels |
| width/height | 1..16384 | Integer pixels |
| blur/shadow/corner_radius | 0..1024 | Integer pixels |
| color | 0..4294967295 | Integer packed color |

Opacity/scale allow fractions; pixels/colors are integral. Color channels
interpolate independently. Descriptive effects are fade/slide/scale/spring/genie;
native code supplies geometry/direction. One-shot event mode adds DWM_LUA_EVENT,
WINDOW and TARGET. Desktop daemon mode sets DWM_LUA_DAEMON=1 and receives bounded
`EVENT <name> <window> <target>` lines via controlled PTY stdin; STOP ends it.
Lines may append stable app/component ID and panel token from committed registry.

Responses are `DWM_EVENT 2 <name> <window> <animation-count>`, ANIMATE records,
optional command/widget/subscription records, then END. ANIMATE contains window,
target, effect, duration, property (none or a scalar), signed 64-bit endpoints
and 33 signed Q16 samples. One-shot consumers require successful exit; daemon
consumers accept a complete bounded batch. SUBSCRIPTIONS mask reporting lets
unsubscribed periodic widgets avoid Lua/protocol traffic. Quotas reset per event.

## Commands and widgets

`dwm.command('launch:gui_terminal')` returns data to the same typed dispatcher as
bindings/buttons. Allowed launcher/close/maximize/minimize/retile/workspace:N/
launch:APP_ID actions revalidate registry/workspace references. At most 32 COMMAND
records follow animations before END. Arbitrary executables/shell/filesystem/
kernel handle operations are unavailable. A malformed record rejects the entire
batch; commands execute only after all records validate.

```lua
dwm.on('widget::updated', function(event)
    if event.component == 'activity' then
        dwm.widget.set(event.target, {text='Ready', color=0xffaabbcc})
    end
end)
```

At most 16 widget updates use current component tokens (index+1) of valid type.
Text is at most 256 UTF8 bytes without controls; empty clears it. Optional ARGB
color overrides text; absence retains it. Wire is
`WIDGET <token> <decimal-color-or--1> <hex-UTF8-or-dash>`, preventing text framing
injection. Atlas updates occur after validating the complete batch. Scene-layer/
panel tokens cannot write widget text.

## Native motion

Component events can animate window=0,target=event.target with scalar properties.
Parent translation/scale/opacity propagate through nested components. Panel token
motion transforms its complete child tree; panel color means background and
component color means text. Tokens expire on scene reload; use current callback
tokens rather than retaining indices.

dwm.targets.launcher/workspace/notifications select native layers with window=0.
Workspace motion transforms rendering and window input coordinates together;
desktop panels retain their own geometry. Closing launcher content remains owned
through completion. Notification opacity/geometry/text/radius/blur/shadow apply
per card. Window geometry commits via configure/ack on completion; manual geometry
interaction cancels tracks. Genie owns a frozen client snapshot while the client
continues updating. Elapsed monotonic time drives motion/dock hover, so dropped
frames cannot extend duration; missing clock finishes immediately. Resizing is
a buffer protocol transaction rather than interpolated client pixels.

## Isolation and limits

The runner offers selected base and pure math/string/table functions plus the
two explicit event modules. It does not open os/io/package/debug/coroutine;
random/randomseed, file/runtime loaders, print/warn, raw/metatable operations,
explicit GC and pcall/xpcall are removed. Real Lua arithmetic/tables/functions/
loops/assert/error/iteration remain. No timers or per-frame callback API is exposed.

`userspace/lua_runtime/limits.h` budgets tracked Lua allocations to 1 MiB,
including aligned headers/state/parsing; source storage/native stack/libc overhead
are outside it. The 100,000-instruction count hook checks every 1,000 instructions,
not compilation/arbitrary C calls. Event limits are 32 animations, 64 handlers
per event, 32 named curves, 32 commands and 16 widgets. Host queue capacity is 32
events; widget timer/hover updates coalesce by target, lifecycle events evict an
update before overflowing. Overflow without evictable update is reported.

This restricted API is not hard CPU/memory sandboxing. Inherited process authority
is unchanged, quotas cover only their stated domains, and the deterministic hash
seed is appropriate for trusted policy rather than hostile input. Parent deadlines
terminate/reap child workers; locally trusted scripts are required. Malformed
results/invalid samples/quota failures discard complete batches and rendering
continues with native fallback. Reload probes candidates in separate bounded
processes before replacing live desktop state.

The layout picker discovers nonempty `.lua` files in the configured layout's
parent; event examples live in scripts/events. Empty drafts cannot execute.
Both paths remain editable. Lua never rewrites privileged configuration.

## Diagnostics

`[diagnostics] performance=true` enables one PERF-SERVER aggregate per 128
completed iterations plus a final partial window. Absent/false performs no
diagnostic clock/stats syscalls or reports. Reload validates it atomically;
Settings-only saves preserve the table. Reporting itself costs time outside
measured stage sums.

Fields ending _ms are window sums: work excludes sleep; pump is client IPC/
texture service; input is device input/policy; compose excludes present;
done is completion/control; sleep is pacing wait. Idle pump can run outside
work, so stages are not exclusive CPU time. Maximum intervals also report.
Counters cover pointer/wheel/loss/key/client IPC records, full/partial/blocked
frames and present attempts/pixels. Sampled ipc_qmax/ready_max come from iteration
SystemStats, not exact per-client peaks or kernel pointer depth.

`[render] frame_interval_ms=16` is reloadable 1..1000 ms. Absolute frame-start
deadlines charge work against pacing; IPC readiness wakes wait-set before the
next compose. FrameDone follows composition; at most 64 pointer records drain
per iteration without merging wheel/button edges. Current HPET supplies monotonic
span time while PIT remains 100 Hz events. Historical PIT-only measurements have
10 ms resolution/IRQ-off undercount; the whole-loop frame marker includes sleep.
See [measurement limits](../performance/measurements.md#measurement-contract) and
[continuous input](../performance/gui-performance.md#continuous-input).

## Verification

Use the existing qemu_test.py harness; serialize builds/VMs sharing artifacts.
Historical gates preserve actual child wait status, exact output/state/pixels,
fault/boot budgets and no partial policy application. Commands are:

```sh
bash tools/test_dwm_host.sh
bash tools/test_dwm_config_host.sh
python3 tools/qemu_test.py --build --markers-file tools/lua_markers.json
python3 tools/qemu_test.py --markers-file tools/lua_desktop_markers.json
python3 tools/qemu_test.py --markers-file tools/dwm_engine_v2_markers.json
python3 tools/qemu_test.py --markers-file tools/settings_ui_markers.json
python3 tools/qemu_test.py --markers-file tools/gui_smoke_markers.json
python3 tools/qemu_test.py --markers-file tools/themes_terminal_markers.json
python3 tools/qemu_test.py --markers-file tools/themes_ui_markers.json --ready-marker 'gui_server: desktop ready' --cmd-timeout 150
python3 tools/qemu_test.py --markers-file tools/svg_markers.json
python3 tools/qemu_test.py --markers-file tools/svg_ui_markers.json
python3 tools/qemu_test.py --markers-file tools/display_3k_markers.json --ready-marker 'gui_server: desktop ready' --cmd-timeout 240 --workdir /tmp/dunit-3k
```

The 3K test clicks 1920×1080 -> 3072×1920 -> 2880×1800 -> 2560×1600 ->
2560×1440 -> 1920×1080; verify PPM dimensions with identify. SVG native tests
cover pixels/VFS at 24/74/256px, bitmaps, wallpaper, cycles and invalid-SVG/no-
bitmap-fallback. Theme tests import all four presets, switch Light/Dark/Nord
through real input and open Files, observing inherited Nord
`bg=0xFF2E3440 accent=0xFF88C0D0`. Fixed override palettes remain smoke fixtures.

Lua child tests cover columns/monocle/zero/environment/repeatability and reject
invalid dimensions/count/source/binary chunks/shape/coordinates/animation,
allocation exhaustion and infinite loops with nonzero status and no policy
records. Native dwm_lua_test bounds PTY polling, kills/reaps failed children and
checks diagnostics; merged stderr/stdout is not an independent stdout proof.
Event tests cover subscriptions/scalars/custom curves/commands/widgets,
persistent state and atomic NaN/invalid data rejection.

Engine tests exercise actual Genie pixels, fan groups, restore, source editing/
long paths, invalid-Lua profile rollback, user-profile import, independent panels,
widget text and launcher/workspace/notification motion. Existing GUI smoke keeps
cross-process buffers/maximize/restore/recycle; Settings tests actual discovered
columns/monocle and acknowledgements. Optional bounded key_interval_ms enables
realistic software-editor typing; pending draft edits drain before repaint.

Sysroot preparation fingerprints actual musl fork files/compiler, including
uncommitted source, and publishes its cache stamp only after a successful rebuild.
This fixes stale libc causing os.remove EINVAL despite a source path-length fix;
the Lua system budget retains deletion/bytecode/modules/input/native TCC rebuild.
See [system Lua](lua-runtime.md#programs-and-commands). No remove assertion or libc adapter was bypassed.

The static policy runner uses the explicit Lua core/lauxlib/base/math/string/table
units shared with lua_spike, cross-TCC C11/O2/Wall/Wextra and the vendored include
path. No host libm/dlopen/readline/external linker or Linux feature macros are
required. Compile alone is not native acceptance. Screenshots/generated products
stay in ignored build output. Prior receipts do not prove complete kernel/GUI
stress stability; the previously observed rare kernel page fault remains a
separate unresolved limit. This documentation edit adds no new native test result.
