# Desktop presets

The production default is Mojave Dark: original vector dune wallpaper, dark
surfaces, top panel, centered bottom dock, left circular window controls and
centered titles. It is a Dunit desktop inspired by the reference, with real
Dunit applications; unavailable services are not drawn as fake menu items.

Settings → Themes discovers TOML presets from `[desktop] themes`. The shipped
presets are Mojave Dark, macOS Light, Nord and Green Tea. The complete overlay
is checked by the same validator used by terminal setup before saving and
requesting live desktop reload. Unknown, invalid and clamped values reject the
whole theme. Applications reopened after the change inherit the desktop colors;
explicit per-app colors in `apps/*.toml` override inheritance. The terminal
keeps its independently configured ANSI palette.

```
exec setup import /assets/dwm/themes/mojave-dark.toml
```

`[layout]` owns `dock_edge` (top/bottom/left/right), `dock_centered`, `dock_w`,
`dock_padding`, `dock_gap`, `dock_margin`, `dock_tile_alpha`, `traffic_lights`, and
`show_workspaces`. `[theme]` owns close/minimize/maximize colors. Rendering,
pointer hit testing and work-area reservation use the same dock geometry.
Green Tea explicitly restores the left dock and legacy window controls.

Themes and layout changes are session RAM only, matching existing settings
storage. The original wallpaper source is `assets/wallpapers/mojave.svg`; the
shipped BMP is rendered from it for the existing bitmap loader.

# Lua stale-libc correction

The previous build reused libc solely because `libc.a` existed. An existing
checkout could therefore link Lua against libc from before the native remove
path-length fix, producing `Invalid argument` on os.remove even with updated
source. `tools/ensure_musl_sysroot.py` now fingerprints actual fork files and the
compiler, rebuilds generated sysroot/libc artifacts on change, and records the
fingerprint only after a successful build. This also includes uncommitted fork
source changes. No Lua assertion is removed and os.remove uses the normal libc
adapter. `tools/lua_system_markers.json` checks deletion, bytecode, modules,
interactive input and a complete TCC rebuild inside Dunit.

# Validation

The QEMU acceptance budgets are `tools/themes_terminal_markers.json` (the full
Lua regression, the exact relative-path command from the reported failure, all
four theme imports and setup validation) and `tools/themes_ui_markers.json`
(real pointer input switches light, dark and Nord, then opens Files). The UI
budget uses `--ready-marker 'gui_server: desktop ready' --cmd-timeout 150`;
the production GUI does not boot the kernel terminal. QMP screenshots capture
the rendered themes. The Files serial diagnostic confirms inherited Nord
colors: `bg=0xFF2E3440 accent=0xFF88C0D0`.

Existing setup, Settings/Lua placement and GUI maximize/restore/recycle smoke
budgets remain enabled. Smoke-only app palettes are explicit fixtures under
`assets/dwm/apps/test`, preserving the existing fixed-color override checks;
production app configs inherit the theme instead. ABI generation checks and
all 13 host DWM tests also pass. These checks do not establish full kernel/GUI
stress stability; the previously recorded rare kernel page fault remains a
separate unresolved issue.
