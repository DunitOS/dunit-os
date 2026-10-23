#!/usr/bin/env python3
"""Pack userspace ELFs + assets + DWM config into a Limine-module initrd archive.

The kernel no longer embeds application binaries or desktop assets via
`include_bytes!`; instead Limine loads this archive as a boot module and the
kernel populates its VFS from it (kernel/src/initrd.rs -> kernel/src/fs/vfs.rs).

Archive format "DUNITRD1" (all integers little-endian, no padding):

    magic:  8 bytes  b"DUNITRD1"
    count:  u32      number of file entries
    entry * count:
        name_len: u32          length of the VFS path in bytes
        data_len: u32          length of the file payload in bytes
        name:     name_len     UTF-8 VFS path, e.g. "/app/calc"
        data:     data_len     raw file bytes

Entries are sorted by path for a deterministic image. Directories are implied
by the paths and recreated by the kernel; they are not stored.
"""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import struct
import sys


MAGIC = b"DUNITRD1"

# Configs that build the kernel with `boot-smoke-tests`; for those the desktop
# default must be test.toml (autostarts gui_clients), otherwise default.toml
# (clean desktop). This rule MUST stay in sync with the Makefile KERNEL_FEATURES
# filter (same two basenames) so the kernel feature and the shipped config agree.
SMOKE_CONFIGS = {"limine_test_terminal.conf", "limine_test_gui.conf"}


def dwm_default_source(assets_dir: Path, config: str | None) -> Path:
    smoke = config is not None and os.path.basename(config) in SMOKE_CONFIGS
    return assets_dir / "dwm" / ("test.toml" if smoke else "default.toml")


def collect(args: argparse.Namespace) -> list[tuple[str, bytes]]:
    entries: dict[str, bytes] = {}

    def add(vfs_path: str, host_path: Path) -> None:
        entries[vfs_path] = host_path.read_bytes()

    userspace = Path(args.userspace_dir)
    for source in sorted(userspace.iterdir()):
        if source.is_file():
            add(f"/app/{source.name}", source)

    assets = Path(args.assets_dir)
    excludes = {e.strip("/") for e in args.exclude}
    for host_path in sorted(p for p in assets.rglob("*") if p.is_file()):
        rel = host_path.relative_to(assets).as_posix()
        if rel in excludes:
            continue
        add(f"/assets/{rel}", host_path)

    # DWM system config tree read by gui_server at /system/share/dwm. default.toml
    # is selected per-config (clean vs smoke); apps/* and widgets/* are globbed
    # generically so no application name is hardcoded in the packer.
    default_src = dwm_default_source(assets, args.config)
    if not default_src.is_file():
        raise RuntimeError(f"missing DWM default config: {default_src}")
    add("/system/share/dwm/default.toml", default_src)
    for sub in ("apps", "widgets"):
        src_dir = assets / "dwm" / sub
        if src_dir.is_dir():
            for host_path in sorted(src_dir.glob("*.toml")):
                add(f"/system/share/dwm/{sub}/{host_path.name}", host_path)
            # Smoke palettes are explicit fixtures, just like test.toml and
            # test service manifests. Production apps inherit the live theme.
            if args.config is not None and os.path.basename(args.config) in SMOKE_CONFIGS:
                for host_path in sorted((src_dir / "test").glob("*.toml")):
                    add(f"/system/share/dwm/{sub}/{host_path.name}", host_path)

    # init service manifests read by /app/init at /system/services. The top-level
    # services/*.toml are the production services (e.g. gui-server.toml); they ship
    # on EVERY config. services/test/*.toml are test-only fixtures (e.g.
    # init-probe.toml, which exits non-zero to exercise init's restart policy) and
    # ship ONLY for the smoke configs, mirroring the SMOKE_CONFIGS test/default
    # selection used for the DWM toml above — the production desktop never sees them.
    if args.services_dir is not None:
        services = Path(args.services_dir)
        for host_path in sorted(services.glob("*.toml")):
            add(f"/system/services/{host_path.name}", host_path)
        smoke = args.config is not None and os.path.basename(args.config) in SMOKE_CONFIGS
        if smoke:
            test_dir = services / "test"
            if test_dir.is_dir():
                for host_path in sorted(test_dir.glob("*.toml")):
                    add(f"/system/services/{host_path.name}", host_path)

    # Dunit sysroot for the on-device C toolchain (tcc). Headers + libc.a + crt
    # go under /system so /app/tcc can compile and link C programs on the device
    # itself. Only packed when --sysroot-dir is given (always, in a normal build).
    if args.sysroot_dir is not None:
        sysroot = Path(args.sysroot_dir)
        inc = sysroot / "include"
        if inc.is_dir():
            for host_path in sorted(p for p in inc.rglob("*") if p.is_file()):
                rel = host_path.relative_to(inc).as_posix()
                add(f"/system/include/{rel}", host_path)
        lib = sysroot / "lib"
        for name in ("libc.a", "crt1.o", "crti.o", "crtn.o"):
            p = lib / name
            if p.is_file():
                add(f"/system/lib/{name}", p)
                # crt also under /system/tcc so tcc finds them via -B/CONFIG_TCCDIR
                if name != "libc.a":
                    add(f"/system/tcc/{name}", p)

    # System Lua is a normal executable; its complete source SDK permits a real
    # on-device rebuild through the same TCC, with no shell/command-size tricks.
    if args.lua_bin is not None:
        add("/system/bin/lua", Path(args.lua_bin))
    if args.luac_bin is not None:
        add("/system/bin/luac", Path(args.luac_bin))
    if args.lua_dir is not None:
        source = Path(args.lua_dir)
        for path in sorted(source.iterdir()):
            if path.suffix in (".c", ".h"):
                add(f"/system/src/lua/{path.name}", path)
    if args.lua_config is not None:
        add("/system/src/lua/dunit-config.h", Path(args.lua_config))
    if args.lua_build_rsp is not None:
        add("/system/src/lua/build.rsp", Path(args.lua_build_rsp))

    # tcc's own runtime (libtcc1.a) and compiler headers (stddef.h/stdarg.h/…)
    # live under /system/tcc = CONFIG_TCCDIR, found by -B/the baked tccdir.
    if args.tcc_dir is not None:
        tccd = Path(args.tcc_dir)
        libtcc1 = tccd / "libtcc1.a"
        if libtcc1.is_file():
            add("/system/tcc/libtcc1.a", libtcc1)
        tinc = tccd / "include"
        if tinc.is_dir():
            for host_path in sorted(p for p in tinc.rglob("*") if p.is_file()):
                rel = host_path.relative_to(tinc).as_posix()
                add(f"/system/tcc/include/{rel}", host_path)

    # The on-device compiler itself: /system/bin/tcc (an OS tool, not a bundled
    # /app application), found by `exec tcc` via the /system/bin exec path.
    if args.tcc_bin is not None:
        tcc_bin = Path(args.tcc_bin)
        if tcc_bin.is_file():
            add("/system/bin/tcc", tcc_bin)

    return sorted(entries.items())


def pack(entries: list[tuple[str, bytes]]) -> bytes:
    out = bytearray()
    out += MAGIC
    out += struct.pack("<I", len(entries))
    for name, data in entries:
        name_bytes = name.encode("utf-8")
        out += struct.pack("<II", len(name_bytes), len(data))
        out += name_bytes
        out += data
    return bytes(out)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--userspace-dir", type=Path, required=True)
    parser.add_argument("--assets-dir", type=Path, required=True)
    parser.add_argument("--services-dir", type=Path, default=None,
                        help="dir of init service manifests packed to /system/services")
    parser.add_argument("--config", type=str, default=None,
                        help="limine config path; selects default vs test DWM toml")
    parser.add_argument("--sysroot-dir", type=Path, default=None,
                        help="Dunit sysroot (include/ + lib/) packed to /system for on-device tcc")
    parser.add_argument("--tcc-dir", type=Path, default=None,
                        help="tcc fork dir (libtcc1.a + include/) packed to /system/tcc")
    parser.add_argument("--lua-bin", type=Path, default=None,
                        help="System Lua executable, built by Dunit cross-TCC")
    parser.add_argument("--luac-bin", type=Path, default=None,
                        help="System Lua bytecode compiler, built by Dunit cross-TCC")
    parser.add_argument("--lua-dir", type=Path, default=None,
                        help="Unmodified Lua C sources/headers for on-device TCC")
    parser.add_argument("--lua-config", type=Path, default=None,
                        help="Dunit Lua installation defaults header")
    parser.add_argument("--lua-build-rsp", type=Path, default=None,
                        help="Generated on-device TCC response file")
    parser.add_argument("--tcc-bin", type=Path, default=None,
                        help="the on-device tcc binary, packed to /system/bin/tcc")
    parser.add_argument("--exclude", action="append", default=[],
                        help="asset path (relative to assets dir) to omit, repeatable")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    entries = collect(args)
    image = pack(entries)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_bytes(image)
    print(f"[INITRD] packed {len(entries)} files -> {args.out} ({len(image)} bytes)")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (RuntimeError, OSError) as error:
        print(f"[INITRD] pack failed: {error}", file=sys.stderr)
        raise SystemExit(1)
