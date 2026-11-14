#!/usr/bin/env python3
"""Actual atomic page tables and canonical copy, with owned host RAM mappings.

Only physical-address translation and the PageFlags wrapper are host shims;
the table definitions, entry operations and kernel-half copy come from VMM.
Hardware walks, CR3 and TLB invalidation require the native AP tests.
"""
import os
from pathlib import Path
import re
import subprocess
import tempfile

from dunit_cli import ROOT


def between(source, start, end):
    if source.count(start) != 1 or source.count(end) != 1:
        raise ValueError(f"VMM extraction markers missing or ambiguous: {start!r}, {end!r}")
    first = source.index(start)
    last = source.index(end)
    if first >= last:
        raise ValueError(f"VMM extraction markers out of order: {start!r}, {end!r}")
    return source[first:last]


def host_source():
    source = (ROOT / "kernel/src/memory/vmm.rs").read_text()
    constants = []
    for name in ("PAGE_SIZE", "PML4_KERNEL_START", "UNINITIALIZED_ROOT"):
        matches = re.findall(rf"^const {name}: usize = [^;]+;", source, re.MULTILINE)
        if len(matches) != 1:
            raise ValueError(f"VMM constant missing or ambiguous: {name}")
        constants.append(matches[0])
    definitions = between(
        source,
        "#[repr(C, align(4096))]\npub struct PageTable {",
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub enum AddressSpaceError",
    )
    canonical_copy = between(
        source,
        "unsafe fn sync_kernel_half_locked(root_frame: usize)",
        "\nunsafe fn read_cr3()",
    )
    fixture = (ROOT / "tools/test_page_tables_host.rs.in").read_text()
    for marker, actual in (
        ("@CONSTANTS@", "\n".join(constants)),
        ("@DEFINITIONS@", definitions),
        ("@CANONICAL_COPY@", canonical_copy),
    ):
        if fixture.count(marker) != 1:
            raise ValueError(f"Fixture marker missing or ambiguous: {marker}")
        fixture = fixture.replace(marker, actual)
    return fixture


if __name__ == "__main__":
    with tempfile.TemporaryDirectory() as folder:
        path = Path(folder) / "page_tables.rs"
        path.write_text(host_source())
        binary = path.with_suffix("")
        subprocess.run(
            [os.environ.get("RUSTC", "rustc"), "--edition=2021", str(path), "-O", "-o", str(binary)],
            check=True,
        )
        subprocess.run([str(binary)], check=True, timeout=60)
