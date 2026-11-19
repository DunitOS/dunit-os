#!/usr/bin/env python3
"""Image composition policy shared by the build, initrd packer and installer.

The full profile is the existing developer/test payload. Release compositions
are declared in images/<name>/profile.json and never depend on leftover files.
"""
from __future__ import annotations
import argparse
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PROFILES = ("full", "minimal", "dwm")


def load_profile(name: str = "full") -> dict:
    if name not in PROFILES:
        raise ValueError(f"unknown image profile: {name}")
    if name == "full":
        return {"name": name, "boot_config": "limine.conf", "apps": None, "assets": ["**/*"],
                "desktop": True, "toolchain": True, "services": None,
                "boot_services": ["netd.toml"], "libexec": ["netd", "netd_fixture", "net_socket_test"]}
    data = json.loads((ROOT / "images" / name / "profile.json").read_text())
    if data.get("name") != name:
        raise ValueError(f"profile name mismatch: {name}")
    basenames = ("apps", "services", "c_apps", "libexec", "boot_services")
    for key in (*basenames, "assets"):
        if key in ("c_apps", "libexec", "boot_services") and key not in data:
            continue
        values = data[key]
        if not isinstance(values, list) or len(set(values)) != len(values):
            raise ValueError(f"invalid profile list: {name}.{key}")
        for value in values:
            if not isinstance(value, str) or not value or value.startswith("/") or ".." in value.split("/"):
                raise ValueError(f"invalid profile path: {name}.{key}")
            if key in basenames and not all(c.isascii() and (c.isalnum() or c in "_-.") for c in value):
                raise ValueError(f"profile entry must be a safe basename: {value}")
    if set(data["apps"] + data.get("c_apps", [])) & set(data.get("libexec", [])):
        raise ValueError(f"boot executable is also declared as a public app: {name}")
    for key in ("desktop", "toolchain"):
        if type(data[key]) is not bool:
            raise ValueError(f"invalid profile flag: {name}.{key}")
    boot = data["boot_config"]
    if not isinstance(boot, str) or not boot.endswith(".conf") or not all(c.isascii() and (c.isalnum() or c in "_-.") for c in boot):
        raise ValueError(f"invalid boot config: {name}")
    size = data["installer_esp_mib"]
    if type(size) is not int or not 64 <= size <= 128:
        raise ValueError(f"invalid installer FAT32 size: {name}")
    return data


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profile", choices=PROFILES)
    parser.add_argument("--field", choices=("apps", "programs", "toolchain", "desktop", "boot_config"))
    args = parser.parse_args()
    profile = load_profile(args.profile)
    if args.field == "programs":
        value = profile["apps"] + profile.get("libexec", []) if profile["apps"] is not None else None
    else:
        value = profile.get(args.field) if args.field else profile
    if isinstance(value, list):
        print(" ".join(value))
    elif isinstance(value, bool):
        print(str(value).lower())
    else:
        print(json.dumps(value) if not isinstance(value, str) else value)


if __name__ == "__main__":
    main()
