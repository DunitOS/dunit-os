# Dunit icon packages

Pure `no_std + alloc` code; no syscalls or rendering policy in this crate.

- `archive::ReadAt` supplies bounded random access. ZIP central directories are
  indexed once; selected files use stored or raw DEFLATE decoding, checked local
  headers, exact output length and CRC32. ZIP64, multidisk archives, encryption,
  symlinks, duplicate/traversal paths and unsupported methods are rejected.
- Directory index limit: 8 MiB / fewer than 65535 entries. The caller bounds each
  compressed and decompressed member. No complete archive extraction is needed.
- `theme::Index` reads freedesktop `index.theme` Directories/ScaledDirectories,
  Fixed/Threshold/Scalable sizes and Scale. Stable distance sorting preserves
  source order for ties. Fallback policy belongs to the VFS manifests; original
  upstream Inherits entries do not implicitly open additional sources.
- `theme::strings` parses the strict string-valued TOML subset used by the theme
  manifests and alias maps. Duplicate keys and malformed string values fail.

`userspace/dunit_image::IconTheme` adapts this to Dunit seek/read/close and the
shared static SVG renderer. A manifest is resolved completely before use. Only
missing icons follow configured fallback; corrupt archives or unsupported SVGs
remain errors. GUI processes cache rasterized icons, so painting has no disk I/O.

Tests cover malformed/truncated archives, CRC errors, traversal, decompression
caps, directory size/scale ordering and duplicate config keys. The package test
reads every member of all five shipped archives and renders all 13 semantic app
and MIME keys at 24/32/74/256 px (the sparse Symbolic package has explicit Dark
fallback, exercised by the native test).

```
cargo test --manifest-path runtime/dunit-icons/Cargo.toml --target x86_64-unknown-linux-gnu
```

The same ZIP decoder runs inside `svg_test`; `tools/svg_markers.json` also checks
all five presets sequentially through the actual settings overlay parser.
