#!/usr/bin/env python3
"""Host regressions for SDK integrity, extraction and relocatable CLI paths."""
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import tarfile
import tempfile
import tomllib
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from dunit_cli import sdk_cargo_options, execution_checkout, create
import dunit_cli

spec = importlib.util.spec_from_file_location('sdk_bundle', Path(__file__).parent / 'sdk/bundle.py')
bundle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bundle)


class Bundle(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.addCleanup(self.temporary.cleanup)

    def archive(self, members):
        path = self.root / 'sdk.tar.gz'
        with tarfile.open(path, 'w:gz') as archive:
            for info, data in members:
                archive.addfile(info, io.BytesIO(data))
        return path

    def file(self, name, data=b'payload', mode=0o644):
        info = tarfile.TarInfo(name)
        info.size = len(data)
        info.mode = mode
        return info, data

    def valid(self):
        content = b'payload'
        manifest = {'schema_version': 1, 'files': {'tools/demo': {
            'sha256': hashlib.sha256(content).hexdigest(), 'size': len(content), 'mode': 0o755}}}
        return [self.file('tools/demo', content, 0o755), self.file(bundle.MANIFEST, json.dumps(manifest).encode())]

    def test_install_verify_and_tamper(self):
        destination = self.root / 'installed'
        bundle.install(self.archive(self.valid()), destination)
        self.assertEqual(bundle.verify(destination), 1)
        (destination / 'tools/demo').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'integrity'):
            bundle.verify(destination)

    def test_install_never_overwrites(self):
        destination = self.root / 'installed'
        destination.mkdir()
        (destination / 'precious').write_text('keep')
        with self.assertRaisesRegex(ValueError, 'exists'):
            bundle.install(self.archive(self.valid()), destination)
        self.assertEqual((destination / 'precious').read_text(), 'keep')

    def test_reject_paths_links_special_files_duplicates_and_permissions(self):
        cases = []
        for name in ('../escape', '/escape', 'a/../escape', 'a//b', './escape', 'a\\b', 'a\nb'):
            cases.append([self.file(name)])
        for kind in (tarfile.SYMTYPE, tarfile.LNKTYPE, tarfile.FIFOTYPE, tarfile.CHRTYPE, tarfile.DIRTYPE):
            info, data = self.file('unsafe', b'')
            info.type = kind
            info.linkname = '../escape'
            cases.append([(info, data)])
        cases += [[self.file('duplicate'), self.file('duplicate')], [self.file('setuid', mode=0o4755)]]
        for members in cases:
            with self.subTest(members=members[0][0].name):
                destination = self.root / 'installed'
                with self.assertRaises(ValueError):
                    bundle.install(self.archive(members), destination)
                self.assertFalse(destination.exists())
                self.assertFalse((self.root / 'escape').exists())
                self.assertEqual(list(self.root.glob('.sdk-install-*')), [])

    def test_reject_missing_manifest_unlisted_files_and_oversized_archive(self):
        for members in ([self.file('only')], self.valid() + [self.file('extra')]):
            with self.assertRaises((OSError, ValueError)):
                bundle.install(self.archive(members), self.root / 'installed')
        with patch.object(bundle, 'MAX_BYTES', 2), self.assertRaisesRegex(ValueError, 'limits'):
            bundle.install(self.archive(self.valid()), self.root / 'installed')
        self.assertFalse((self.root / 'installed').exists())

    def test_verify_rejects_symlinks_and_permission_changes(self):
        destination = self.root / 'installed'
        bundle.install(self.archive(self.valid()), destination)
        (destination / 'tools/demo').chmod(0o644)
        with self.assertRaisesRegex(ValueError, 'integrity'):
            bundle.verify(destination)
        (destination / 'linked').symlink_to('/etc/passwd')
        with self.assertRaisesRegex(ValueError, 'special'):
            bundle.verify(destination)

    def test_registry_snapshot_is_seeded_into_external_project(self):
        sdk = self.root / 'moved-sdk'
        snapshot = sdk / 'cargo-registry/index/test/.cache'
        snapshot.mkdir(parents=True)
        (snapshot / 'real-index-entry').write_bytes(b'index fixture')
        external = self.root / 'outside-project'
        external.mkdir()
        with patch.object(dunit_cli, 'ROOT', sdk), patch.object(dunit_cli, 'STANDALONE', True), patch.dict('os.environ', {}, clear=True):
            dunit_cli.configure_sdk_cargo_home(external / 'Cargo.toml')
            import os
            cargo_home = Path(os.environ['CARGO_HOME'])
            self.assertEqual(cargo_home, external / 'target/.dunit-cargo-home')
            self.assertEqual((cargo_home / 'registry/index/test/.cache/real-index-entry').read_bytes(), b'index fixture')
        self.assertEqual(list(sdk.iterdir()), [sdk / 'cargo-registry'])

    def test_native_delegation_preserves_checkout_cache_and_owns_target_paths(self):
        sdk = self.root / 'sdk'
        (sdk / 'cargo-registry').mkdir(parents=True)
        external = self.root / 'external'
        external.mkdir()
        checkout = self.root / 'checkout'
        checkout.mkdir()
        artifact = external / 'demo'
        artifact.write_bytes(b'ELF fixture')
        manifest = external / 'Cargo.toml'
        app = SimpleNamespace(kind='cli', binary='demo', id='demo', name='Demo',
                              capabilities=(), icon=None)
        caller = dict(os.environ, CARGO_HOME='/original/checkout-cache',
                      CARGO_TARGET_DIR='/external/application-target',
                      RUSTUP_HOME='/original/toolchain')
        def application_build(*args):
            self.assertEqual(os.environ['CARGO_HOME'], str(external / 'target/.dunit-cargo-home'))
            self.assertEqual(os.environ['CARGO_TARGET_DIR'], caller['CARGO_TARGET_DIR'])
            return artifact
        with (
            patch.dict(os.environ, caller, clear=True),
            patch.object(dunit_cli, 'ROOT', sdk),
            patch.object(dunit_cli, 'STANDALONE', True),
            patch.object(dunit_cli, 'execution_checkout', return_value=checkout),
            patch.object(dunit_cli, 'project', return_value=(manifest, {}, app)),
            patch.object(dunit_cli, 'build', side_effect=application_build),
            patch.object(dunit_cli, 'checked') as checked,
        ):
            self.assertEqual(dunit_cli.main(['run', '--manifest-path', str(manifest)]), 0)
        self.assertEqual(checked.call_count, 2)
        for call in checked.call_args_list:
            environment = call.kwargs['env']
            self.assertEqual(environment['CARGO_HOME'], caller['CARGO_HOME'])
            self.assertEqual(environment['RUSTUP_HOME'], caller['RUSTUP_HOME'])
            self.assertEqual(environment['RUSTUP_TOOLCHAIN'], 'nightly')
            self.assertNotIn('CARGO_TARGET_DIR', environment)
        self.assertEqual(checked.call_args_list[0].args[0][1], 'check')
        self.assertEqual(checked.call_args_list[0].kwargs['cwd'], checkout / 'kernel')
        self.assertIn(str(checkout / 'tools/qemu_test.py'), checked.call_args_list[1].args[0])
        self.assertIn('--build', checked.call_args_list[1].args[0])
        self.assertEqual(checked.call_args_list[1].kwargs['cwd'], checkout)

    def test_standalone_starters_use_explicit_relative_sdk_path_without_patches(self):
        for kind in ('cli', 'tui', 'gui'):
            destination = self.root / ('starter_' + kind)
            args = SimpleNamespace(path=destination, id=None, name=None, kind=kind)
            with patch.object(dunit_cli, 'STANDALONE', True):
                create(args)
            manifest = destination / 'Cargo.toml'
            dependency = tomllib.loads(manifest.read_text())['dependencies']['dunit']
            self.assertFalse(Path(dependency['path']).is_absolute())
            self.assertEqual((destination / dependency['path']).resolve(), dunit_cli.ROOT / 'runtime/dunit')
            self.assertNotIn('version', dependency)
            if kind == 'gui':
                self.assertEqual(dependency['features'], ['gui'])
            sdk = self.root / 'layout'
            sdk.mkdir(exist_ok=True)
            (sdk / 'sdk-layout.json').write_text(json.dumps({'local_crates': {'dunit': {'path': 'runtime/dunit'}}}))
            with patch.object(dunit_cli, 'ROOT', sdk), patch.object(dunit_cli, 'STANDALONE', True):
                self.assertEqual(sdk_cargo_options(manifest), ['--offline'])

    def test_patch_only_explicit_sdk_roots_including_gui_and_renamed_optional(self):
        crates = {
            'dunit': {'path': 'runtime/dunit'},
            'dunit_render': {'path': 'runtime/dunit-render'},
            'dunit_c': {'path': 'runtime/dunit-c'},
        }
        (self.root / 'sdk-layout.json').write_text(json.dumps({'local_crates': crates}))
        manifest = self.root / 'Cargo.toml'
        for dependency in (
            'dunit = "=0.1.0"',
            'dunit = { version = "=0.1.0", features = ["gui"] }',
            'native = { package = "dunit", version = "=0.1.0", optional = true }',
        ):
            with self.subTest(dependency=dependency):
                manifest.write_text('[dependencies]\n' + dependency + '\n')
                with patch.object(dunit_cli, 'ROOT', self.root), patch.object(dunit_cli, 'STANDALONE', True):
                    options = sdk_cargo_options(manifest)
                self.assertEqual(len(options), 3)
                self.assertEqual(options[0:2], ['--offline', '--config'])
                self.assertIn('patch.crates-io."dunit".path=', options[2])
                self.assertNotIn('dunit-render', options[2])
        manifest.write_text('[dependencies]\ndunit = { path = "already-local" }\nserde = "1"\n')
        with patch.object(dunit_cli, 'ROOT', self.root), patch.object(dunit_cli, 'STANDALONE', True):
            self.assertEqual(sdk_cargo_options(manifest), ['--offline'])

    def test_cli_paths_are_resolved_after_sdk_move(self):
        (self.root / 'sdk-layout.json').write_text(json.dumps({'local_crates': {'dunit': {'path': 'runtime/dunit', 'version': '0.1.0'}}}))
        with patch.object(dunit_cli, 'ROOT', self.root), patch.object(dunit_cli, 'STANDALONE', True):
            manifest = self.root / "Cargo.toml"
            manifest.write_text('[dependencies]\ndunit = "=0.1.0"\n')
            options = sdk_cargo_options(manifest)
            self.assertIn('--offline', options)
            self.assertNotIn('source.crates-io.replace-with', ' '.join(options))
            self.assertTrue(any(str(self.root / 'runtime/dunit') in option for option in options))
            with patch.dict('os.environ', {}, clear=True), self.assertRaisesRegex(ValueError, 'DUNIT_OS_CHECKOUT'):
                execution_checkout()


if __name__ == '__main__':
    unittest.main()
