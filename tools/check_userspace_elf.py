#!/usr/bin/env python3
"""Validate the static ELF profile accepted by Dunit Userspace ABI v0.

This is a release/build check, not a substitute for the kernel ELF validator.
No host executable or target code is executed by this module.
"""
import argparse
from pathlib import Path
import struct
import sys


class InvalidElf(ValueError):
    pass


def check(data, allowed_osabi=(0,)):
    def require(condition, reason):
        if not condition:
            raise InvalidElf(reason)

    def region(offset, size):
        require(offset <= len(data) and size <= len(data) - offset,
                "table or segment extends beyond file")
        return data[offset:offset + size]

    require(len(data) >= 64, "truncated ELF header")
    require(data[:7] == b"\x7fELF\x02\x01\x01", "expected ELF64 little-endian version 1")
    require(data[7] in allowed_osabi and data[8] == 0, "unsupported ELF OSABI/version")
    (kind, machine, version, entry, phoff, shoff, flags, ehsize, phsize,
     phnum, shsize, shnum, shstr) = struct.unpack_from("<HHIQQQIHHHHHH", data, 16)
    require(kind == 2, "expected static ET_EXEC")
    require(machine == 62 and version == 1 and flags == 0, "expected x86_64 ABI v0")
    require(ehsize == 64 and phsize == 56 and 0 < phnum < 65535,
            "invalid/extended program-header layout")
    region(phoff, phsize * phnum)
    entry_ok = False
    stack = False
    loads = 0
    for i in range(phnum):
        typ, pf, off, va, _, fs, ms, align = struct.unpack_from("<IIQQQQQQ", data, phoff+i*56)
        region(off, fs)
        require(typ != 3, "PT_INTERP is outside the static profile")
        require(typ != 2, "PT_DYNAMIC/NEEDED is outside the static profile")
        if typ == 0x6474e551:
            stack = True
            require(not pf & 1, "executable GNU_STACK")
        if typ == 1:
            loads += 1
            require(fs <= ms, "LOAD filesz exceeds memsz")
            require(not (pf & 1 and pf & 2), "writable executable LOAD")
            require(align in (0, 1) or (align & (align-1) == 0 and va % align == off % align),
                    "invalid LOAD alignment")
            require(va < (1 << 47) and ms <= (1 << 47)-va, "LOAD outside userspace")
            if pf & 1 and va <= entry < va + fs:
                entry_ok = True
    require(loads and entry_ok, "entry is not in a file-backed executable LOAD")
    require(stack, "missing explicit non-executable GNU_STACK")
    if shoff == 0:
        require(shnum == 0, "section count without section headers")
        return {"loads": loads, "symbols": 0}
    require(shsize == 64 and shnum > 0, "invalid/extended section-header layout")
    region(shoff, shnum * shsize)
    sections = [struct.unpack_from("<IIQQQQIIQQ", data, shoff+i*64) for i in range(shnum)]
    symbols = 0
    for sec in sections:
        _, typ, _, _, off, size, link, _, _, entsize = sec
        if typ != 8:  # NOBITS has no bytes in the file.
            region(off, size)
        require(typ != 6, "dynamic section/NEEDED outside static profile")
        if typ not in (2, 11):
            continue
        require(entsize == 24 and size % 24 == 0 and link < shnum, "invalid symbol table")
        strings_sec = sections[link]
        require(strings_sec[1] == 3, "symbol table does not link to a string table")
        strings = region(strings_sec[4], strings_sec[5])
        for pos in range(off, off+size, 24):
            name, info, _, index, _, _ = struct.unpack_from("<IBBHQQ", data, pos)
            require(name < len(strings), "symbol name outside string table")
            end = strings.find(b"\0", name)
            require(end >= 0, "unterminated symbol name")
            if name and index == 0 and info >> 4 in (1, 2):
                raise InvalidElf("undefined symbol: " + strings[name:end].decode("utf8", "replace"))
            symbols += 1
    return {"loads": loads, "symbols": symbols}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("files", type=Path, nargs="+")
    parser.add_argument("--allowed-osabi", type=int, action="append", default=None,
                        help="allowed ELF identification OSABI (default: System V=0)")
    args = parser.parse_args()
    failed = False
    for path in args.files:
        try:
            result = check(path.read_bytes(), args.allowed_osabi or (0,))
            print(f"[ELF] OK {path} loads={result['loads']} symbols={result['symbols']}")
        except (OSError, InvalidElf, struct.error) as error:
            failed = True
            print(f"[ELF] FAIL {path}: {error}", file=sys.stderr)
    return int(failed)


if __name__ == "__main__":
    raise SystemExit(main())
