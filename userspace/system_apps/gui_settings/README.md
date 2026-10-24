# GUI Settings

GUI Settings is a compositor-launched gui-v1 client, separate from the terminal
`setup` application. A sidebar groups the existing live controls:

- Appearance: all seventeen theme color slots, including inactive borders and
  title text, with vertical scrolling through the full palette. GUI arrows cycle
  palette swatches; terminal `setup set theme.FIELD "#RRGGBB"` accepts arbitrary
  hexadecimal colors.
- Effects: blur, gradient, animations, corner radius, shadow and animation duration.
- Desktop geometry: panel height, title height and dock width.
- Display: keep-boot-mode and resolution presets through 3072×1920, including
  2560×1440, 2560×1600 and 2880×1800. Actual availability depends on the display
  backend and video memory. Bochs/QEMU can grow beyond the boot framebuffer;
  fixed Limine/GOP backends retain their boot mode.
- Window behavior: enable/disable the Lua window policy, choose a discovered
  layout script, and adjust its runner timeout from 50 to 5000 milliseconds.
- Themes: discover regular `.toml` files in `desktop.themes`, validate a complete
  overlay, then save and request compositor reload. The shipped theme names are
  data in that folder; the picker has no compiled list. Invalid values, unknown
  settings and incomplete reads reject the entire overlay and keep the current
  settings. Theme overlays preserve the application registry and unrelated tables
  through the common settings save path. The theme directory is editable in setup.

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
keeps its boot mode. This GUI exposes the controls listed above, not every typed
setting exposed by terminal setup. Window behavior edits the existing typed
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
This GUI does not edit script source or implement a separate animation engine.

Color swatches come from the TOML file selected by desktop.palette, not a
compiled color array. Change that path in setup and reopen Settings to load a
different swatch set. An unreadable palette is reported; current theme colors
remain available. Arbitrary current colors are retained in the selection list.

Terminal import and GUI theme import share the same checked scalar edit and
overlay validator in `dwm_settings::overlay`; each complete file is limited to
64 KiB. Color and resolution pickers resynchronize after a successful import.
