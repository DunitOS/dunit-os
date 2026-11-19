#!/usr/bin/env python3
"""Check actual generated/public headers on fresh and cached sysroots."""
import os
from pathlib import Path
import subprocess
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
        for name in ('syscall','errno','rights','transfer','packet','socket'):
            self.assertIn(f'#include <dunit/{name}.h>',source)
        self.assertNotRegex(source,r'#define\s+(?:SYS_|DUNIT_(?:E[A-Z]|RIGHT_|TRANSFER_))')
    def test_fresh_install(self):
        sysroot_tool.install_headers(self.root,self.sysroot);self.assert_installed()
    def test_network_layout_in_installed_umbrella(self):
        sysroot_tool.install_headers(self.root, self.sysroot)
        subprocess.run(
            ['cc', '-std=c11', '-Wall', '-Wextra', '-Werror',
             '-I', str(self.sysroot / 'include'), '-xc', '-fsyntax-only', '-'],
            input='''
#include <dunit.h>
_Static_assert(SYS_NET_DEVICE_GRANT == 95, "grant syscall");
_Static_assert(SYS_NET_QUEUE_OPEN == 96, "open syscall");
_Static_assert(DUNIT_RIGHT_NET_CONTROL == (1u << 7), "control right");
_Static_assert(DUNIT_RIGHT_NET_RAW == (1u << 8), "raw right");
_Static_assert(sizeof(struct dunit_packet_options) == 16, "options length");
_Static_assert(sizeof(struct dunit_packet_info) == 64, "info length");
_Static_assert(sizeof(struct dunit_packet_control) == 16, "control length");
_Static_assert(sizeof(struct dunit_packet_frame) == 8, "frame header length");
_Static_assert(offsetof(struct dunit_packet_info, mac) == 8, "MAC");
_Static_assert(offsetof(struct dunit_packet_info, rx_frames) == 16, "RX frames");
_Static_assert(offsetof(struct dunit_packet_info, tx_errors) == 56, "TX errors");
_Static_assert(SYS_NET_SERVICE_REGISTER == 97, "service register syscall");
_Static_assert(SYS_NET_CONNECT_GRANT == 98, "connect grant syscall");
_Static_assert(SYS_NET_SOCKET_OPEN == 99, "socket open syscall");
_Static_assert(SYS_NET_SOCKET_OP == 100, "socket operation syscall");
_Static_assert(DUNIT_RIGHT_NET_CONNECT == (1u << 9), "connect right");
_Static_assert(DUNIT_RIGHT_NET_SERVICE == (1u << 10), "service right");
_Static_assert(sizeof(struct dunit_socket_options) == 8, "socket options length");
_Static_assert(sizeof(struct dunit_socket_endpoint) == 8, "socket endpoint length");
_Static_assert(sizeof(struct dunit_socket_receive) == 16, "datagram metadata length");
_Static_assert(sizeof(struct dunit_socket_endpoints) == 16, "endpoint pair length");
_Static_assert(sizeof(struct dunit_socket_bridge) == 40, "socket bridge header length");
_Static_assert(offsetof(struct dunit_socket_bridge, kind) == 16, "bridge kind");
_Static_assert(offsetof(struct dunit_socket_bridge, ipv4) == 24, "bridge IPv4");
_Static_assert(offsetof(struct dunit_socket_bridge, length) == 32, "bridge payload length");
''',
            text=True, check=True,
        )
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
