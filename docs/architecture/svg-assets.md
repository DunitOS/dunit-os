# SVG GUI assets

The GUI loads vector assets from the VFS at runtime. `runtime/dunit-svg` is a
pure no-std renderer; `userspace/dunit_image` provides one bounded, checked file
loader for both the compositor and the file manager. No GUI policy or SVG
parser is added to the kernel and no syscall/protocol changes are required.

For an application registry icon stem, the loader first tries
`/assets/icons/<desktop.icon_theme>/<stem>.svg`, then the existing 32×32
`.rgba` format only when the SVG is absent. Absolute icon and logo paths can
also select SVG. A present but malformed/unsupported SVG is reported and is
not silently replaced by a bitmap. Missing assets retain the existing initials
or primitive fallback, keeping the desktop usable.

The compositor rasterizes SVG icons at its configured dock size plus the hover
margin and caches the resulting dimensions and pixels. Launcher, task switcher,
panel and dock blits use those dimensions; they no longer assume every image
is 32×32. Theme/layout reload rebuilds the cache. File-manager mimetype SVGs
render at the client's icon size through the same loader.

SVG wallpapers render once at the actual framebuffer dimensions on startup,
mode change and reload; transparent pixels composite over the configured
desktop background. The Mojave source SVG replaces the unused pre-rendered BMP.
Existing bitmap wallpapers and Breeze RGBA icons remain supported for Green Tea
and existing user configurations. `desktop.icon_theme`, application `icon`,
`desktop.logo` and `desktop.wallpaper` remain configuration choices.

The default/Mojave, Light and Nord presets select the original Aqua SVG set.
Green Tea explicitly selects the existing Breeze set. The 13 editable Aqua
sources and their MIT license live in `assets/icons/aqua`. The supported static
SVG profile and resource limits are documented in `runtime/dunit-svg/README.md`;
this is an asset renderer, not a full browser SVG implementation.

Validation uses `tools/qemu_test.py`: `tools/svg_markers.json` executes native
pixel checks and VFS loading at 24/74/256px, bitmap compatibility, SVG wallpaper,
local-cycle rejection and refusal to fall back from invalid SVG even when a
valid bitmap exists. `tools/svg_ui_markers.json` exercises real production theme
switching and Files, with QMP screenshots. Existing GUI smoke and 3K mode
switching also verify integration, including framebuffer-sized SVG wallpapers.
