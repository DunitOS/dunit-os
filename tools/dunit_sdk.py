#!/usr/bin/env python3
"""Generate relocatable-build-system cross files for a selected Dunit sysroot.

Run after ensure_musl_sysroot.py. Generated files refer to the selected absolute
sysroot; regenerate them after moving it. They never run target executables.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shlex
import subprocess
from string import Template


def generate(sysroot, output):
    sysroot = Path(sysroot).resolve(strict=True)
    output = Path(output).resolve()
    required = ["bin/x86_64-dunit-cc", "bin/x86_64-dunit-ar", "bin/x86_64-dunit-strip",
                "include/stdio.h", "lib/libc.a", "lib/crt1.o", "lib/crti.o", "lib/crtn.o"]
    for name in required:
        if not (sysroot / name).is_file():
            raise ValueError(f"incomplete Dunit sysroot: {sysroot / name}")
    if any(c in str(sysroot) for c in "\n\r\x00"):
        raise ValueError("sysroot path contains a control character")
    if "'''" in str(sysroot):
        raise ValueError("sysroot path contains Meson triple-quote delimiter")
    root = str(sysroot)
    values = {"cmake_root": root.replace("\\", "\\\\").replace('"', '\\"').replace("$", "\\$"),
              "meson_root": root,
              "shell_root": shlex.quote(root)}
    templates = Path(__file__).resolve().parent / "sdk/templates"
    output.mkdir(parents=True, exist_ok=True)
    generated = []
    for filename in ("toolchain.cmake", "meson.cross", "config.site"):
        source = (templates / (filename + ".in")).read_text()
        (output / filename).write_text(Template(source).substitute(values))
        generated.append(filename)
    metadata = {"target": "x86_64-dunit", "abi": "Dunit Userspace ABI v0",
                "profile": "static", "sysroot": root, "generated": generated,
                "libc_sha256": hashlib.sha256((sysroot / "lib/libc.a").read_bytes()).hexdigest()}
    repository = Path(__file__).resolve().parents[1]
    abi_files = sorted((repository / "abi").glob("*.abi"))
    metadata["abi_sha256"] = hashlib.sha256(b"".join(
        path.name.encode() + b"\0" + path.read_bytes() for path in abi_files)).hexdigest()
    metadata["sources"] = {}
    for name in ("dunit-musl", "dunit-tcc"):
        source = repository / "toolchains" / name
        revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=source, text=True).strip()
        status = subprocess.check_output(["git", "status", "--porcelain"], cwd=source)
        names = subprocess.check_output(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], cwd=source).split(b"\0")
        digest = hashlib.sha256()
        for item in sorted(set(names)):
            if item:
                path = source / item.decode()
                if path.is_file(): digest.update(item + b"\0" + path.read_bytes())
        metadata["sources"][name] = {"commit": revision, "dirty": bool(status), "tree_sha256": digest.hexdigest()}
    metadata["unsupported"] = {
        "M8": ["socket objects", "TCP/IP", "DNS"],
        "separate_contracts": ["signals/interruption", "dynamic loader/TLS", "canonical termios", "secure entropy source"],
        "explicit_refusal": ["async/deferred pthread cancellation", "unsupported spawn attributes", "unsupported open/ioctl flags"],
    }
    metadata["supported"] = [
        "static x86_64 native ELF/crt/argv/env/TLS", "bounded scalar and buffered file I/O",
        "private anonymous and eager file mmap; W^X", "pipe/dup/fcntl/poll native subset",
        "native spawn ordered open/close/dup2/chdir actions", "monotonic and boot-UTC clock",
        "pthread create/join/mutex/cond/barrier subset", "typed PTY geometry/input-count ioctl",
    ]
    portable = {key: value for key, value in metadata.items() if key != "sysroot" and key != "generated"}
    (output / "profile.json").write_text(json.dumps(portable, indent=2) + "\n")
    (output / "musl-COPYRIGHT").write_bytes((repository / "toolchains/dunit-musl/COPYRIGHT").read_bytes())
    (output / "sdk.json").write_text(json.dumps(metadata, indent=2) + "\n")
    return metadata


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sysroot", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        metadata = generate(args.sysroot, args.output)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    print(f"[SDK] generated {', '.join(metadata['generated'])} in {args.output}")


if __name__ == "__main__":
    main()
