#!/usr/bin/env python3
"""Host regressions for profile selection and the unchanged DUNITRD1 archive."""
import argparse
from pathlib import Path
import tempfile
import unittest
from image_profile import load_profile
from pack_initrd import collect, pack
from check_image_profile import read_archive, verify_entries


class ImageProfiles(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.args = argparse.Namespace(profile="minimal", userspace_dir=self.root / "apps",
            assets_dir=self.root / "assets", services_dir=self.root / "services",
            config="limine_minimal.conf", exclude=[], sysroot_dir=self.root / "sysroot",
            tcc_dir=None, tcc_bin=None, lua_bin=None, luac_bin=None, lua_dir=None,
            lua_config=None, lua_build_rsp=None)
        for path, data in {
            "assets/exec-path": b"/system/bin:/app\n",
            "assets/dwm/default.toml": b"desktop", "assets/dwm/session.toml": b"session",
            "assets/dwm/test.toml": b"test desktop", "assets/fonts/font.ttf": b"font",
            "assets/images/demo.gif": b"large demo",
            "services/gui-server.toml": b'exec = "/app/gui_server"\n',
            "services/test/probe.toml": b"probe", "sysroot/include/header.h": b"header",
        }.items():
            self.write(path, data)
        profile = load_profile("dwm")
        for app in profile["apps"] + profile["c_apps"] + ["test_only"]:
            self.write("apps/" + app, app.encode())

    def write(self, path, data):
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)

    def test_minimal_drops_stale_gui_tests_services_and_toolchain(self):
        entries = dict(collect(self.args))
        verify_entries(entries, "minimal")
        self.assertEqual(entries, read_archive(pack(sorted(entries.items()))))
        self.assertNotIn("/app/gui_server", entries)
        self.assertNotIn("/system/include/header.h", entries)
        self.assertNotIn("/assets/fonts/font.ttf", entries)

    def test_missing_required_app_fails_instead_of_shipping_partial_image(self):
        (self.args.userspace_dir / "dsh").unlink()
        with self.assertRaisesRegex(RuntimeError, "missing required minimal application"):
            collect(self.args)

    def test_dwm_has_only_declared_apps_and_assets(self):
        self.args.profile = "dwm"
        self.args.config = "limine_dwm.conf"
        entries = dict(collect(self.args))
        for path in ("/app/gui_server", "/app/dwm_lua", "/system/share/dwm/default.toml", "/system/include/header.h"):
            self.assertIn(path, entries)
        for path in ("/app/test_only", "/assets/images/demo.gif", "/system/services/probe.toml"):
            self.assertNotIn(path, entries)

    def test_dwm_verifier_rejects_unavailable_registry_app(self):
        self.args.profile = "dwm"
        self.args.config = "limine_dwm.conf"
        self.write("toolchain/tcc", b"compiler")
        self.write("toolchain/lua", b"interpreter")
        self.args.tcc_bin = self.root / "toolchain/tcc"
        self.args.lua_bin = self.root / "toolchain/lua"
        self.write("assets/dwm/default.toml", b'[application.gui_server]\nexec = "/app/gui_server"\n')
        entries = dict(collect(self.args))
        verify_entries(entries, "dwm")
        entries["/system/share/dwm/default.toml"] = b'[application.absent]\n'
        with self.assertRaisesRegex(ValueError, "configured desktop application missing"):
            verify_entries(entries, "dwm")

    def test_full_developer_profile_keeps_existing_smoke_fixtures(self):
        self.args.profile = "full"
        self.args.config = "limine_test_terminal.conf"
        entries = dict(collect(self.args))
        self.assertIn("/app/test_only", entries)
        self.assertIn("/system/services/probe.toml", entries)
        self.assertEqual(entries["/system/share/dwm/default.toml"], b"test desktop")

    def test_verifier_rejects_payload_leaks_and_wrong_metadata(self):
        entries = dict(collect(self.args))
        entries["/app/gui_server"] = b"unexpected"
        with self.assertRaises(ValueError):
            verify_entries(entries, "minimal")
        del entries["/app/gui_server"]
        entries["/system/share/dunit-image.json"] = b'{}'
        with self.assertRaises(ValueError):
            verify_entries(entries, "minimal")

    def test_archive_rejects_corruption_and_duplicate_paths(self):
        valid = pack([("/app/a", b"a")])
        for data in (valid[:-1], valid + b"trailing", b"invalid", pack([("/a", b"a"), ("/a", b"b")])):
            with self.assertRaises(ValueError):
                read_archive(data)


if __name__ == "__main__":
    unittest.main()
