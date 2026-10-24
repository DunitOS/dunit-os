#!/usr/bin/env python3
"""Host tests: synthetic ELF rejection and generated cross-file contracts."""
import json
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import unittest

from check_userspace_elf import InvalidElf, check
from dunit_sdk import generate


def elf():
    data = bytearray(1024)
    data[:16] = b"\x7fELF\x02\x01\x01" + bytes(9)
    struct.pack_into("<HHIQQQIHHHHHH", data, 16,
                     2, 62, 1, 0x400200, 64, 768, 0, 64, 56, 2, 64, 3, 0)
    struct.pack_into("<IIQQQQQQ", data, 64, 1, 5, 512, 0x400200, 0, 16, 16, 1)
    struct.pack_into("<IIQQQQQQ", data, 120, 0x6474e551, 6, 0, 0, 0, 0, 0, 1)
    data[600:607] = b"\0entry\0"
    struct.pack_into("<IBBHQQ", data, 664, 1, 0x12, 0, 1, 0x400200, 16)
    struct.pack_into("<IIQQQQIIQQ", data, 832, 0, 3, 0, 0, 600, 7, 0, 0, 1, 0)
    struct.pack_into("<IIQQQQIIQQ", data, 896, 0, 2, 0, 0, 640, 48, 1, 0, 8, 24)
    return data


class ElfTests(unittest.TestCase):
    def test_static_profile(self):
        self.assertEqual(check(elf()), {"loads": 1, "symbols": 2})

    def reject(self, offset, fmt, value, reason):
        data = elf()
        struct.pack_into(fmt, data, offset, value)
        with self.assertRaisesRegex(InvalidElf, reason):
            check(data)

    def test_class_endian_machine_type(self):
        for offset, fmt, value, reason in [(4,"B",1,"ELF64"), (5,"B",2,"ELF64"),
                (18,"H",3,"x86_64"), (16,"H",3,"ET_EXEC"),
                (7,"B",3,"OSABI"), (8,"B",1,"OSABI")]:
            with self.subTest(offset=offset):
                self.reject(offset, fmt, value, reason)

    def test_selected_osabi(self):
        data = elf()
        data[7] = 3
        self.assertEqual(check(data, (3,))["loads"], 1)

    def test_dynamic_interpreter_wx_stack_undefined(self):
        for offset, fmt, value, reason in [(64,"I",3,"INTERP"), (64,"I",2,"DYNAMIC"),
                (68,"I",7,"writable executable"), (124,"I",7,"GNU_STACK"),
                (670,"H",0,"undefined symbol"), (120,"I",0,"missing explicit")]:
            with self.subTest(reason=reason):
                self.reject(offset, fmt, value, reason)

    def test_bounds_and_entry(self):
        for offset, fmt, value, reason in [(32,"Q",1023,"beyond file"),
                (24,"Q",0,"entry"), (96,"Q",17,"filesz"),
                (112,"Q",3,"alignment"), (920,"Q",9999,"beyond file"),
                (664,"I",99,"symbol name")]:
            with self.subTest(reason=reason):
                self.reject(offset, fmt, value, reason)

    def test_truncation_and_empty(self):
        for count in (0, 63, 100, 700, 950):
            with self.subTest(count=count), self.assertRaises(InvalidElf):
                check(elf()[:count])


class SdkTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "sys root's $value"
        for name in ("bin/x86_64-dunit-cc", "bin/x86_64-dunit-ar", "bin/x86_64-dunit-strip",
                     "include/stdio.h", "lib/libc.a", "lib/crt1.o", "lib/crti.o", "lib/crtn.o"):
            path = self.root/name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"fixture")
        self.output = Path(self.temp.name)/"cross"

    def test_incomplete_sysroot(self):
        (self.root/"lib/libc.a").unlink()
        with self.assertRaisesRegex(ValueError, "incomplete"):
            generate(self.root, self.output)

    def test_manifest_determinism(self):
        first = generate(self.root, self.output)
        before = {p.name: p.read_bytes() for p in self.output.iterdir()}
        self.assertEqual(generate(self.root, self.output), first)
        self.assertEqual(before, {p.name: p.read_bytes() for p in self.output.iterdir()})
        self.assertEqual(json.loads((self.output/"sdk.json").read_text())["target"], "x86_64-dunit")

    @unittest.skipUnless(shutil.which("cmake"), "CMake unavailable")
    def test_cmake_root_and_find_isolation(self):
        generate(self.root, self.output)
        probe = self.output/"probe.cmake"
        probe.write_text('include("${CMAKE_CURRENT_LIST_DIR}/toolchain.cmake")\n'
                         'file(WRITE "${CMAKE_CURRENT_LIST_DIR}/root.txt" "${DUNIT_SYSROOT}")\n'
                         'foreach(kind LIBRARY INCLUDE PACKAGE)\n'
                         ' if(NOT CMAKE_FIND_ROOT_PATH_MODE_${kind} STREQUAL "ONLY")\n'
                         ' message(FATAL_ERROR "host search leaked")\n endif()\nendforeach()\n')
        subprocess.run(["cmake", "-P", str(probe)], check=True, capture_output=True)
        self.assertEqual((self.output/"root.txt").read_text(), str(self.root))

    def test_shell_site_quoting(self):
        generate(self.root, self.output)
        result = subprocess.check_output(["sh", "-c",
            '. "$1"; printf "%s\\n%s\\n" "$CC" "$PKG_CONFIG_LIBDIR"',
            "sh", str(self.output/"config.site")], text=True).splitlines()
        self.assertEqual(result[0], str(self.root/"bin/x86_64-dunit-cc"))
        self.assertNotIn("/usr", result[1])

    def test_meson_syntax(self):
        try:
            from mesonbuild.machinefile import parse_machine_files
        except ImportError:
            self.skipTest("Meson Python module unavailable")
        generate(self.root, self.output)
        parsed = parse_machine_files([str(self.output/"meson.cross")], str(self.output))
        self.assertEqual(parsed["binaries"]["c"], str(self.root/"bin/x86_64-dunit-cc"))
        self.assertTrue(parsed["properties"]["needs_exe_wrapper"])
        self.assertEqual(parsed["host_machine"]["system"], "dunit")


if __name__ == "__main__":
    unittest.main()
