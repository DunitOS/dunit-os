#!/usr/bin/env python3
"""Verify the Live ISO and its embedded installer carry the same image profile."""
from __future__ import annotations
import argparse
import json
from pathlib import Path
import struct
import subprocess
import tempfile
import tomllib
from image_profile import ROOT, load_profile


def read_archive(data: bytes) -> dict[str, bytes]:
    if len(data) < 12 or data[:8] != b"DUNITRD1":
        raise ValueError("invalid initrd header")
    count, = struct.unpack_from("<I", data, 8)
    offset = 12
    entries = {}
    for _ in range(count):
        if offset + 8 > len(data):
            raise ValueError("truncated initrd entry")
        namesize, size = struct.unpack_from("<II", data, offset)
        offset += 8
        if not namesize or offset + namesize + size > len(data):
            raise ValueError("truncated initrd payload")
        name = data[offset:offset + namesize].decode("utf-8")
        offset += namesize
        if not name.startswith("/") or ".." in name.split("/") or name in entries:
            raise ValueError(f"invalid/duplicate initrd path: {name}")
        entries[name] = data[offset:offset + size]
        offset += size
    if offset != len(data):
        raise ValueError("trailing initrd bytes")
    return entries


def verify_entries(entries: dict[str, bytes], name: str) -> None:
    profile = load_profile(name)
    if json.loads(entries["/system/share/dunit-image.json"]) != profile:
        raise ValueError("image metadata does not match the selected profile")
    expected = {f"/app/{app}" for app in profile["apps"] + profile.get("c_apps", [])}
    actual = {path for path in entries if path.startswith("/app/")}
    if actual != expected:
        raise ValueError(f"application mismatch: missing={expected-actual}, unexpected={actual-expected}")
    services = {path for path in entries if path.startswith("/system/services/")}
    if services != {f"/system/services/{service}" for service in profile["services"]}:
        raise ValueError("service manifests do not match the profile")
    if not profile["desktop"]:
        allowed = expected | {"/assets/exec-path", "/system/share/dunit-image.json"}
        if set(entries) != allowed:
            raise ValueError(f"unexpected Minimal payload: {set(entries)-allowed}")
    else:
        for required in ("/system/share/dwm/default.toml", "/system/bin/tcc", "/system/bin/lua"):
            if required not in entries:
                raise ValueError(f"missing desktop payload: {required}")
    if profile["desktop"]:
        desktop = tomllib.loads(entries["/system/share/dwm/default.toml"].decode())
        for app_id, app in desktop.get("application", {}).items():
            executable = app.get("exec", f"/app/{app_id}")
            if executable not in entries:
                raise ValueError(f"configured desktop application missing: {executable}")
    for path in services:
        for line in entries[path].decode().splitlines():
            if line.strip().startswith("exec ="):
                executable = line.split("=", 1)[1].strip().strip('"')
                if executable not in entries:
                    raise ValueError(f"service executable missing: {executable}")


def check_iso(iso: Path, profile: str) -> None:
    with tempfile.TemporaryDirectory(prefix="dunit-image-") as folder:
        dest = Path(folder)
        for source, output in (("/boot/initrd.img", "initrd.img"),
                               ("/boot/limine/limine.conf", "limine.conf"),
                               ("/boot/installer-esp.img", "installer.img")):
            subprocess.run(["xorriso", "-osirrox", "on", "-indev", str(iso),
                            "-extract", source, str(dest / output)], check=True,
                           stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        live = (dest / "initrd.img").read_bytes()
        entries = read_archive(live)
        verify_entries(entries, profile)
        if (dest / "limine.conf").read_bytes() != (ROOT / load_profile(profile)["boot_config"]).read_bytes():
            raise ValueError("release boot configuration mismatch")
        subprocess.run(["mcopy", "-i", str(dest / "installer.img"),
                        "::/boot/initrd.img", str(dest / "installed-initrd.img")], check=True)
        if (dest / "installed-initrd.img").read_bytes() != live:
            raise ValueError("Live and installed image payloads differ")
        print(f"[IMAGE] {iso.name}: {profile} verified; {len(entries)} files, "
              f"{len(live)} initrd bytes; installer payload matches")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("iso", type=Path)
    parser.add_argument("profile", choices=("dwm", "minimal"))
    args = parser.parse_args()
    check_iso(args.iso, args.profile)
