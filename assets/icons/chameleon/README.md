# Complete Chameleon icon packages

All five requested upstream sets are included in full, with original SVG bytes,
relative paths, `index.theme`, caches and other supplied resources. Every ZIP
also contains the upstream AUTHORS, GPL license and README. No icon was selected
out, converted to RGBA, recolored or normalized.

| Package | Original theme files | SVG files |
| --- | ---: | ---: |
| Breeze Chameleon Dark | 16227 | 16205 |
| Breeze Chameleon Light | 14613 | 14588 |
| Breeze-Round-Chameleon Dark | 14156 | 14122 |
| Breeze-Round-Chameleon Light | 13582 | 13545 |
| Chameleon-Symbolic-Dark-Icons | 60 | 58 |

Total: 58638 theme files, including 58518 SVG files. The shared upstream AUTHORS,
LICENSE and README make the complete source snapshot 58641 files. Package entry
counts include the three shared documents copied into each ZIP.

Upstream: https://github.com/L4ki/Breeze-Chameleon-Icons
Pinned revision: `39ca2a55ba5289a96b2e744cd3670e020fb7e217`.
The author links this repository to all five KDE Store/Pling pages; those links
are retained in `sources.json` and `catalog.json`. See `LICENSE` and `AUTHORS`.

Packages ship at `/assets/icons/chameleon/*.zip` in the OS. They occupy about
83 MiB compressed instead of 220 MiB expanded. Keeping them packaged avoids
mounting nearly 59000 currently unused MemFS entries and fits the existing
128 MiB installer ESP payload without changing the filesystem, initrd format,
partition policy or kernel.

All five packages are runtime-selectable now. `[desktop] icon_theme` selects a
manifest under `/assets/icons/<name>/theme.toml`; `[files] icon_theme` optionally
overrides it for Files. The shared userspace loader reads the ZIP directory once
per loaded theme, seeks to the requested entry, bounds decompression and verifies
its CRC32. It never expands the complete set in MemFS. Original `index.theme`
directories, sizes, scales and size ranges select the nearest SVG variant.
Explicit `fallback` in each manifest defines the source chain. The upstream
`Inherits` names are not auto-mounted: this keeps fallback sources explicit and
allows Symbolic to inherit the complete Dark package rather than the small
legacy Breeze set. Missing entries fall back; damaged/unsupported SVGs report an
error and do not fall back silently.

| Desktop preset | Icon theme |
| --- | --- |
| default / Mojave Dark | breeze-round-chameleon-dark |
| macOS Light | breeze-round-chameleon-light |
| Nord | breeze-chameleon-dark |
| Green Tea | breeze-chameleon-light |
| Symbolic Dark (icon-only overlay) | chameleon-symbolic-dark |

Example desktop configuration:

```toml
[desktop]
icon_theme = "breeze-chameleon-dark"

[application.gui_files]
icon = "system-file-manager"

[application.gui_terminal]
icon = "apps/48/utilities-terminal.svg"
```

`icon` accepts an original freedesktop name, archive-relative SVG path, or absolute
VFS SVG/RGBA path. Existing semantic app and MIME keys are mapped in
`/assets/icons/chameleon/aliases.toml`; edit this file to assign other icons.
Manifest `archive`, `root`, `aliases`, `fallback` are string keys under `[theme]`.
Manifests and alias maps use the strict string-valued TOML subset (quoted strings,
no escapes); malformed, unknown or duplicate keys reject the manifest. Aqua and
legacy Breeze directory themes remain available by setting `icon_theme` to
`aqua` or `breeze`.

The SVG renderer implements a bounded static profile, not the complete browser
SVG standard. Common dock and file-type icons in all five packages are tested at
24/32/74/256 px. Other archive entries are accessible by name/path, but SVG features
outside that profile (e.g. text, embedded images, masks, clip paths and compound
filter graphs) produce a diagnostic. See `runtime/dunit-svg/README.md`.

`catalog.json` records source and archive counts, sizes, SHA-256 and CRC32.
`catalog.tsv` is used by `verify.lua` to check native file I/O over every stored
archive byte plus ZIP directory bounds/counts. Host import also decompresses and
compares every entry against the pinned upstream source. Reproduce with:

```
python3 tools/import_icon_themes.py --source /path/to/pinned/upstream --destination assets/icons/chameleon
```

Verify in Dunit using `exec lua /assets/icons/chameleon/verify.lua`, or through
`tools/qemu_test.py --build --markers-file tools/icon_packs_markers.json`.
