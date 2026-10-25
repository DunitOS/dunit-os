# Configuration & Lua Engine 2.0 — implementation report

The existing userspace compositor now resolves legacy and schema-2 documents
into one typed component tree. Panel count, edge, contents, nesting, sizing,
styles, app lists and folder presentation come from configuration. The Frost
example uses one full-width bottom surface; the multi-panel example adds an
independent unreserved auto-hidden utility panel. Fixed panel/dock/tray drawing
and bitmap 3×5 shell text were removed. The GUI wire protocol and kernel ABI
remain unchanged.

- Configuration and policy: [schema](architecture/dwm-shell.md).
- Events, animation descriptors, widgets, isolation and limits: [Lua SDK](DWM_LUA_SDK.md).
- Complete examples: `assets/dwm/frost-desktop.toml`,
  `assets/dwm/multi-panel-desktop.toml`, `assets/dwm/styles/frost.dss`,
  `assets/dwm/scripts/events/macos.lua`.
- Settings: source-preserving draft editors, separate theme/desktop import,
  discovered profiles and checked user-profile activation.
- Native rendering: bounded owned snapshots for Genie, scalar tracks, DSS
  transitions, geometry-aware input and existing configure acknowledgements.
- Reload: prepare complete scene and font cache, probe candidate Lua in a
  separate bounded process, then replace committed state. Rejection preserves
  the live scene. Legacy hot reload remains the entry point.

## Verification

`tools/test_dwm_host.sh` and `tools/test_dwm_config_host.sh` pass against production
validation, source preservation, parser, curve, scene motion, pixel composition,
snapshot lifetime and editor code. Target checks for compositor, Settings and
setup pass. The compositor target check has no warnings.

QEMU runs through `tools/qemu_test.py`:

- `tools/lua_markers.json`: PASS. Existing layout regressions plus events,
  all scalar properties, sampled custom spring, commands, widgets, persistent
  state and rejection of invalid curve/property/command/widget data.
- `tools/dwm_engine_v2_markers.json`: PASS. Real launch/minimize keys, actual
  Genie pixels, fan folder click, icon restore, Settings navigation, long path
  input, erroneous Lua profile rejection, import of a user profile, multiple
  panels, Lua widget updates and launcher/workspace/notification scalar animations.
- `tools/gui_smoke_markers.json`: PASS. Two untrusted clients, terminal and files
  share the compositor; maximize/restore configure and acknowledgements pass.
- `tools/settings_ui_markers.json`: PASS. Production dock launch, palette
  scrolling, fixed navigation, enabling Lua, configure acknowledgement and
  switching columns/monocle through discovered layout scripts.

Observed engine serial excerpts:

```text
gui_server: shell group=folder open entries=3 presentation=fan
gui_server: restored pid=55
gui_server: settings reload rejected (Lua preflight Lua event worker exited) — keeping last good
gui_server: shell panel=utility edge=top rect=846,12,166,64 reserve=0 autohide=1
gui_server: widget updated component=activity text=Lua 2
gui_server: component animation committed target=513
gui_server: component animation committed target=514
gui_server: component animation committed target=515
```

The input harness retains its terminal default. A bounded optional
`key_interval_ms` permits realistic typing into software-rendered editors.
Settings also drains pending draft edits before repainting the full editor.
Screenshots are produced in ignored `build/engine-v2-*.ppm`; they are test
artifacts and are not committed.

## Current limits

- Rendering is software based; large blur regions, animations and many surfaces
  depend on CPU and memory budgets. Counts are bounded (64 panels, 512 components,
  depth 16), rather than literally unlimited.
- Genie animates an owned frozen snapshot, while the client continues running.
- The clock uses the current system uptime source; RTC/calendar/time-zone support
  is outside this compositor change.
- Complete configuration uses the existing checked single-line TOML subset.
- Persistence across reboot requires a mounted persistent `/persist` filesystem;
  a Live ISO can use a RAM-backed path.
- Settings editors expose validated source fields rather than a drag-and-drop
  panel designer. Global font paths and component font sizes are configurable.
- Existing gui-v1, tiling runner and schema-1 import remain supported.

## Changed files

User edits to the old theme and clock widget, and the user's Frost theme/script,
were preserved and excluded from these changes.

- `assets/dwm/frost-desktop.toml`
- `assets/dwm/multi-panel-desktop.toml`
- `assets/dwm/scripts/events/macos.lua`
- `assets/dwm/scripts/tests/error-events.lua`
- `assets/dwm/scripts/tests/event-commands.lua`
- `assets/dwm/scripts/tests/event-component.lua`
- `assets/dwm/scripts/tests/event-invalid-command.lua`
- `assets/dwm/scripts/tests/event-invalid-property.lua`
- `assets/dwm/scripts/tests/event-invalid-widget.lua`
- `assets/dwm/scripts/tests/event-invalid.lua`
- `assets/dwm/scripts/tests/event-properties.lua`
- `assets/dwm/scripts/tests/event-property.lua`
- `assets/dwm/scripts/tests/event-spring.lua`
- `assets/dwm/scripts/tests/event-state.lua`
- `assets/dwm/scripts/tests/event-widget.lua`
- `assets/dwm/scripts/tests/events.lua`
- `assets/dwm/styles/frost.dss`
- `assets/dwm/tests/engine-v2.toml`
- `assets/dwm/tests/invalid-events-desktop.toml`
- `docs/DWM_ENGINE_V2_REPORT.md`
- `docs/DWM_LUA_SDK.md`
- `docs/architecture/dwm-shell.md`
- `tools/dwm_engine_v2_markers.json`
- `tools/gui_boot_markers.json`
- `tools/gui_smoke_markers.json`
- `tools/lua_markers.json`
- `tools/qemu_test.py`
- `tools/test_dwm_config_host.sh`
- `tools/test_dwm_host.sh`
- `userspace/ctests/dwm_lua_test.c`
- `userspace/dwm_settings/src/lib.rs`
- `userspace/dwm_settings/src/policy.rs`
- `userspace/dwm_settings/src/shell.rs`
- `userspace/dwm_settings/src/validation.rs`
- `userspace/dwm_settings/tests/preserve.rs`
- `userspace/dwm_settings/tests/validation.rs`
- `userspace/system_apps/dwm_lua/main.c`
- `userspace/system_apps/dwm_lua/sdk.h`
- `userspace/system_apps/gui_server/Cargo.lock`
- `userspace/system_apps/gui_server/Cargo.toml`
- `userspace/system_apps/gui_server/src/animation.rs`
- `userspace/system_apps/gui_server/src/composite.rs`
- `userspace/system_apps/gui_server/src/desktop_actions.rs`
- `userspace/system_apps/gui_server/src/desktop_motion.rs`
- `userspace/system_apps/gui_server/src/drawing.rs`
- `userspace/system_apps/gui_server/src/lua_policy.rs`
- `userspace/system_apps/gui_server/src/main.rs`
- `userspace/system_apps/gui_server/src/protocol_runtime.rs`
- `userspace/system_apps/gui_server/src/shell_geometry.rs`
- `userspace/system_apps/gui_server/src/shell_layout.rs`
- `userspace/system_apps/gui_server/src/shell_motion.rs`
- `userspace/system_apps/gui_server/src/shell_motion_tests.rs`
- `userspace/system_apps/gui_server/src/shell_resources.rs`
- `userspace/system_apps/gui_server/src/shell_runtime.rs`
- `userspace/system_apps/gui_server/src/shell_text.rs`
- `userspace/system_apps/gui_server/src/window_effects.rs`
- `userspace/system_apps/gui_server/src/window_effects_tests.rs`
- `userspace/system_apps/gui_server/src/window_painter.rs`
- `userspace/system_apps/gui_settings/README.md`
- `userspace/system_apps/gui_settings/src/document_editor.rs`
- `userspace/system_apps/gui_settings/src/main.rs`
- `userspace/system_apps/setup/src/main.rs`
