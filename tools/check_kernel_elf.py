#!/usr/bin/env python3
"""Reject file-backed zero-fill holes in the kernel's ELF load segments."""

import argparse
from pathlib import Path
import struct


def validate(path):
    data = Path(path).read_bytes()
    if data[:6] != b"\x7fELF\x02\x01":
        raise ValueError("expected ELF64 little endian")
    phoff, shoff = struct.unpack_from("<QQ", data, 32)
    phsize, phnum, shsize, shnum, strings = struct.unpack_from("<HHHHH", data, 54)
    if phsize != 56 or shsize != 64:
        raise ValueError("invalid ELF header sizes")
    sections = [
        struct.unpack_from("<IIQQQQIIQQ", data, shoff + i * shsize)
        for i in range(shnum)
    ]
    names = sections[strings]
    table = data[names[4] : names[4] + names[5]]
    named = {table[s[0] :].split(b"\0", 1)[0].decode(): s for s in sections}
    bss = named[".bss"]
    if bss[1] != 8:
        raise ValueError(".bss must be NOBITS")
    segments = [
        struct.unpack_from("<IIQQQQQQ", data, phoff + i * phsize) for i in range(phnum)
    ]
    containing = [
        p
        for p in segments
        if p[0] == 1 and p[3] <= bss[3] and bss[3] + bss[5] <= p[3] + p[6]
    ]
    if len(containing) != 1:
        raise ValueError("BSS must be contained in one LOAD")
    p = containing[0]
    if p[3] + p[5] > bss[3]:
        raise ValueError("LOAD file bytes cover BSS; place GOT/data before NOLOAD BSS")
    if any(v[0] == 1 and v[5] > v[6] for v in segments):
        raise ValueError("invalid LOAD file/memory sizes")
    return {
        "file_bytes": len(data),
        "bss_bytes": bss[5],
        "data_file_bytes": p[5],
        "data_memory_bytes": p[6],
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("elf", type=Path)
    args = parser.parse_args()
    print("[KERNEL-ELF]", validate(args.elf))
