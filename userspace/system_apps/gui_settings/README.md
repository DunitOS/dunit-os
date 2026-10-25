# GUI Settings

GUI Settings is a compositor-launched gui-v1 client, separate from the terminal
`setup` application. A sidebar groups the existing live controls:

- Appearance: all seventeen theme color slots, including inactive borders and
  title text, with vertical scrolling through the full palette. GUI arrows cycle
  palette swatches; terminal `setup set theme.FIELD "#RRGGBB"` accepts arbitrary
  hexadecimal colors.
- Animations: blur, gradient, animation enablement, corner radius, shadow and
  animation duration. The native compositor and Lua event descriptors execute effects.
- Desktop geometry: panel height, title height and dock width.
- Display: keep-boot-mode and resolution presets through 3072×1920, including
  2560×1440, 2560×1600 and 2880×1800. Actual availability depends on the display
  backend and video memory. Bochs/QEMU can grow beyond the boot framebuffer;
  fixed Limine/GOP backends retain their boot mode.
- Lua Scripts: enable/disable the Lua window policy, choose a discovered
  layout script, and adjust its runner timeout from 50 to 5000 milliseconds.
- Themes: discover regular `.toml` files in `desktop.themes`, validate a complete
  overlay, then save and request compositor reload. The shipped theme names are
  data in that folder; the picker has no compiled list. Invalid values, unknown
  settings and incomplete reads reject the entire overlay and keep the current
  settings. Theme overlays preserve the application registry and unrelated tables
  through the common settings save path. The separate **Full desktop** button
  validates and replaces the complete document, including panels, components,
  bindings, rules and the application registry; registry-free overlays are rejected
  by that button. The theme directory is editable in setup.

The interface uses the current configured theme for its background, sidebar,
buttons, selection, borders and text. It follows both light and dark palettes;
the content uses the configured menu background and the sidebar uses the panel
background. Changing sidebar pages
changes only the view and does not write or reload desktop configuration.
Control changes use the existing settings save and compositor reload path.
A failed save restores the previous settings and picker positions.

The client handles compositor CONFIGURE events using fresh shared buffers and
protocol buffer IDs, and retires its previous protocol buffer. Large surfaces
expand the background; small surfaces expose the full form through wheel
scrolling and horizontal/vertical footer arrows. The footer remains visible and
reports save failures, rollback and reload-request failures inside the window.

Current limits: configuration storage is session RAM through
`dwm_settings`. Display mode changes depend on the backend; a fixed framebuffer
keeps its boot mode. Panels, Typography, Widgets, Keybindings, Window Rules and Lua event hook expose
source-preserving configuration editors. Lua Scripts edits the existing typed
`scripts` configuration and requests compositor reload through the same save and
rollback path as other controls. The compositor executes the selected policy
asynchronously; enabling a policy does not guarantee successful execution.

The layout picker lists regular `.lua` files from the parent directory of the
configured `scripts.layout` path, refreshed when its page opens. It does not
hardcode the supplied example names. Safe ASCII filenames are selectable only
when their full paths round-trip through the shared configuration model; folders,
unsafe names and paths too long for that model are excluded. Read errors and an
empty directory are shown in the page. Use terminal `setup` to change the script
folder (by editing `scripts.layout`) or the read-only `scripts.runner` path.
The Lua event hook page edits the `[scripts]` table, including `events` and runner
paths. This GUI does not edit Lua script source or implement an animation engine.

Color swatches come from the TOML file selected by desktop.palette, not a
compiled color array. Change that path in setup and reopen Settings to load a
different swatch set. An unreadable palette is reported; current theme colors
remain available. Arbitrary current colors are retained in the selection list.

Terminal import and GUI theme import share the same checked scalar edit and
overlay validator in `dwm_settings::overlay`; each complete file is limited to
64 KiB. Color and resolution pickers resynchronize after a successful import.


The configuration editors operate on a draft of the actual desktop document.
Panels shows `[panels.ID]`; Widgets shows `[components.ID]`; Keybindings shows
`[bindings.ID]`; Window Rules shows `[rules.ID]`. Typography shows `desktop.font`
and the panel/title font sizes in `[layout]`. These pages share a single draft,
so cross-references can be edited together before applying them.

Click a line and type using the compositor's cooked ASCII keyboard input.
Left/Right, Home/End, Backspace and Delete respect UTF-8 boundaries in existing
text. `+ line` inserts a field after the selected line; `+ table` inserts a named
table for panels/components/bindings/rules. Replace the proposed identifier and
field text with the desired TOML. `- line` removes the selected line. Table and
field names are validated by the shared settings schema on Apply. Incomplete
headings remain visible while they are being edited. Page arrows limit the
visible list to twelve rows, and the bottom arrows scroll small windows.

**Apply** validates the entire document, including references and resource
limits, before saving and requesting compositor reload. Validation failures show
the line, field and reason in the footer and retain the draft and saved config.
If another application changed the saved document since the draft was loaded,
Apply rejects the stale draft to avoid overwriting those changes. **Revert**
reloads the current saved document. Switching pages retains unsaved changes;
ordinary immediate controls refresh an existing clean draft after saving.

The pure editor regressions are in `src/document_editor.rs` and cover source
preservation, UTF-8 editing, insertion/removal, and incomplete headings. Native
compilation uses the same target and build-std flags as the Makefile; production
behavior remains covered by the shared QEMU GUI regression flow.


Desktop profiles is a separate import mode. It discovers validated complete
`.toml` documents in `/assets/dwm`, including the shipped Frost desktop and
multi-panel examples. Choosing a profile only selects its path. **Import
complete desktop** validates the file again and replaces the complete desktop
configuration, then requests reload. The editable path field also accepts user
profile files at any absolute VFS path; click it and use the same ASCII input
and cursor keys as the document editor. Theme overlays remain on Themes with
their existing controls and positions. Import failures include the filename,
line, field and reason in the footer and serial log.
