# Terminal setup

`setup` is a terminal application, separate from `gui_settings`. It uses the
same `dwm_settings` parser and serializer as the desktop. Run `setup` for an
interactive numbered section/field menu. Choose a section, choose a field,
then enter its new TOML value. Back, Save, Quit and Commands choices remain
available. The `menu` command returns to this menu. `show` lists the supported sections and scalar keys;
`show effects` narrows the output. Edits remain staged until `save`.

```
get effects.anim_ms
set effects.anim_ms 180
set effects.anim false
set theme.desktop "#FF1E1E2E"
check
save
quit
```

A command can also be supplied as process arguments (`setup get effects.anim_ms`,
`setup set effects.anim_ms 180`). A successful argument-mode `set` saves
immediately. Command errors exit with status 1. Interactive errors leave the
session open and preserve staged settings. Unknown fields, ignored input and
values that the parser would clamp are rejected rather than silently accepted.
Schema metadata is read-only. Save verifies the serialized settings by reading
them back and preserves application/startup tables through the shared save API.

This editor covers the typed scalar `Settings` model. It does not edit the
application registry, per-app configuration files or Lua scripts. Settings use
the storage provided by `dwm_settings`; changes currently apply to the next
desktop session, and this program does not claim to trigger a live reload.

## Importing a supplied theme

From setup's command prompt:

```
import /assets/dwm/themes/macos-light.toml
show theme
check
save
```

Interactive import validates the entire overlay, stages it for review, and
requires `save` to write. In process-argument mode,
`setup import /assets/dwm/themes/macos-light.toml` validates and saves immediately,
just like argument-mode `set`. Import errors leave the staged settings unchanged.
Overlay paths must be absolute without parent traversal. Malformed lines,
unknown settings, ignored/clamped values, read errors and files over 64 KiB are
rejected. Colors accept both `#RRGGBB` and `#AARRGGBB`.
