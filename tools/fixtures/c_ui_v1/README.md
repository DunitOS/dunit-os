# Frozen C UI v1 client

`gui.c` and `include/dunit/ui.h` are exact snapshots from `9df23f6`, the first
published C callback window. Do not regenerate them from current sources.
The manifest pins both byte sequences and their original commit. These are
intentional compatibility fixtures, not a second production implementation.

The compatibility runner compiles the old client before rebuilding native Rust,
retains its ELF object unchanged, then asks on-device TCC to link only that
object against the freshly built archive. No compiled object is committed.
