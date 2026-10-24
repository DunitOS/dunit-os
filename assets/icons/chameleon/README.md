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
partition policy or kernel. Existing GUI settings and Aqua/Breeze icons are
unchanged. These packages are not yet expanded into runtime-selectable icon
themes; SVG theme resolution/rendering will be integrated separately.

`catalog.json` records source and archive counts, sizes, SHA-256 and CRC32.
`catalog.tsv` is used by `verify.lua` to check native file I/O over every stored
archive byte plus ZIP directory bounds/counts. Host import also decompresses and
compares every entry against the pinned upstream source. Reproduce with:

```
python3 tools/import_icon_themes.py --source /path/to/pinned/upstream --destination assets/icons/chameleon
```

Verify in Dunit using `exec lua /assets/icons/chameleon/verify.lua`, or through
`tools/qemu_test.py --build --markers-file tools/icon_packs_markers.json`.
