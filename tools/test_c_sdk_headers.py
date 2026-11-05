#!/usr/bin/env python3
"""Check actual generated/public headers on fresh and cached sysroots."""
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import ensure_musl_sysroot as sysroot_tool
from gen_abi import ROOT, render_c_api

class Headers(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name)
        self.source=self.root/'abi/include';self.source.mkdir(parents=True)
        self.headers=list((ROOT/'abi/include').rglob('*.h'))
        for source in self.headers:
            target=self.source/source.relative_to(ROOT/'abi/include')
            target.parent.mkdir(parents=True,exist_ok=True);target.write_bytes(source.read_bytes())
        self.sysroot=self.root/'build/dunit-sysroot'
    def assert_installed(self):
        for source in self.headers:
            relative=source.relative_to(ROOT/'abi/include')
            self.assertEqual((self.sysroot/'include'/relative).read_bytes(),source.read_bytes())
    def test_generated_umbrella_has_no_copied_definitions(self):
        source=(ROOT/'abi/include/dunit.h').read_text()
        self.assertEqual(source,render_c_api())
        for name in ('syscall','errno','rights','transfer'):
            self.assertIn(f'#include <dunit/{name}.h>',source)
        self.assertNotRegex(source,r'#define\s+(?:SYS_|DUNIT_(?:E[A-Z]|RIGHT_|TRANSFER_))')
    def test_fresh_install(self):
        sysroot_tool.install_headers(self.root,self.sysroot);self.assert_installed()
    def test_changed_header_updates_without_touching_other_headers(self):
        sysroot_tool.install_headers(self.root,self.sysroot)
        stable=self.sysroot/'include/dunit/errno.h';os.utime(stable,ns=(1,1))
        changed=self.source/'dunit.h';changed.write_text(changed.read_text()+'/* changed */\n')
        sysroot_tool.install_headers(self.root,self.sysroot)
        self.assertEqual((self.sysroot/'include/dunit.h').read_bytes(),changed.read_bytes())
        self.assertEqual(stable.stat().st_mtime_ns,1)
    def test_cache_hit_still_installs_headers(self):
        (self.sysroot/'lib').mkdir(parents=True)
        (self.sysroot/'lib/libc.a').write_bytes(b'cached')
        (self.sysroot/'.dunit-source-fingerprint').write_text('cached\n')
        with patch.object(sysroot_tool,'__file__',str(self.root/'tools/ensure_musl_sysroot.py')),patch.object(sysroot_tool,'fingerprint',return_value='cached'),patch.object(sysroot_tool.subprocess,'run') as build:
            sysroot_tool.main();build.assert_not_called()
        self.assert_installed()
    def test_rebuild_installs_headers_after_libc(self):
        def rebuild(*args,**kwargs):
            (self.sysroot/'lib').mkdir(parents=True)
            (self.sysroot/'lib/libc.a').write_bytes(b'fresh')
        with patch.object(sysroot_tool,'__file__',str(self.root/'tools/ensure_musl_sysroot.py')),patch.object(sysroot_tool,'fingerprint',return_value='fresh'),patch.object(sysroot_tool.subprocess,'run',side_effect=rebuild) as build:
            sysroot_tool.main();build.assert_called_once()
        self.assert_installed()
        self.assertEqual((self.sysroot/'.dunit-source-fingerprint').read_text(),'fresh\n')

if __name__=='__main__':unittest.main()
