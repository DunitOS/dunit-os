#!/usr/bin/env python3
"""Host-side application contract and generation regressions; native cases run in QEMU."""
import argparse
import json
from pathlib import Path
import tempfile
import unittest

from dunit_cli import ROOT, create
from dunit_manifest import load_manifest, parse_manifest, installed_manifest


class Manifest(unittest.TestCase):
    def setUp(self):
        self.data = dict(schema_version=1, application=dict(
            id="demo", name="Demo", kind="cli", capabilities=[]))

    def parse(self, data=None, bins=None):
        data = self.data if data is None else data
        # JSON values are also TOML values in this restricted fixture.
        text = f'schema_version = {json.dumps(data["schema_version"])}\n'
        text += ''.join(f'{k} = {json.dumps(v)}\n' for k, v in data.items() if k not in ('schema_version', 'application'))
        text += '[application]\n'
        text += ''.join(f'{k} = {json.dumps(v, ensure_ascii=False)}\n' for k, v in data['application'].items())
        return parse_manifest(text, Path("dunit.toml"), ["demo"] if bins is None else bins, ROOT / "abi/rights.abi")

    def test_resolves_exact_target_and_keeps_rights_declarative(self):
        self.data['application']['capabilities'] = ['READ', 'WRITE']
        app = self.parse()
        self.assertEqual(app.binary, 'demo')
        self.assertEqual(app.capabilities, ('READ', 'WRITE'))
        self.data['application']['binary'] = 'other'
        self.assertEqual(self.parse(bins=['demo', 'other']).binary, 'other')

    def test_shared_host_and_guest_contract_fixtures(self):
        for path in (ROOT / 'runtime/dunit-app-manifest/tests/fixtures').glob('*.toml'):
            with self.subTest(path=path):
                if path.name.endswith('.valid.toml'):
                    load_manifest(path, ['gui_stat'], ROOT / 'abi/rights.abi')
                else:
                    with self.assertRaises(ValueError):
                        load_manifest(path, ['gui_stat'], ROOT / 'abi/rights.abi')

    def test_installed_metadata_materializes_cargo_binary(self):
        app = self.parse(bins=['demo'])
        content = installed_manifest(app)
        self.assertIn('binary = "demo"', content)
        self.assertEqual(parse_manifest(content, Path('installed.toml'), ['demo'], ROOT / 'abi/rights.abi'),
                         type(app)(Path('installed.toml'), app.id, app.name, app.kind, app.binary, app.icon, app.capabilities))

    def test_rejects_missing_ambiguous_or_unsafe_binary(self):
        for bins in ([], ['a', 'b'], ['../demo']):
            with self.subTest(bins=bins), self.assertRaisesRegex(ValueError, 'application.binary'):
                self.parse(bins=bins)

    def test_rejects_invalid_fields_with_diagnostics(self):
        for field, values in {
            'id': ['', '../demo', 'UPPER', 'a' * 65, 'á'],
            'name': ['', ' ', 'bad\nname', 123],
            'kind': ['widget', 123],
            'capabilities': [['ROOT'], ['READ', 'READ'], 'READ', [1]],
            'icon': ['relative.svg', '/a/../b', '/a//b', '//a', '/', 123],
        }.items():
            for value in values:
                with self.subTest(field=field, value=value):
                    previous = self.data['application'].copy()
                    self.data['application'][field] = value
                    with self.assertRaisesRegex(ValueError, f'dunit.toml: application.{field}'):
                        self.parse()
                    self.data['application'] = previous
        self.data['application']['kind'] = 'gui'
        with self.assertRaisesRegex(ValueError, 'application.icon'):
            self.parse()
        self.data['application']['icon'] = '/assets/demo.svg'
        self.assertEqual(self.parse().kind, 'gui')

    def test_rejects_schema_and_unknown_keys(self):
        for version in (0, 2, True, '1'):
            self.data['schema_version'] = version
            with self.assertRaisesRegex(ValueError, 'schema_version'):
                self.parse()
        self.data['schema_version'] = 1
        self.data['extra'] = 1
        with self.assertRaisesRegex(ValueError, 'root: unknown keys'):
            self.parse()
        del self.data['extra']
        self.data['application']['extra'] = 1
        with self.assertRaisesRegex(ValueError, 'application: unknown keys'):
            self.parse()

    def test_missing_required_tables_and_fields(self):
        for content in ('', 'schema_version = 1', 'schema_version = 1\napplication = 4'):
            with self.assertRaises(ValueError):
                parse_manifest(content, Path('dunit.toml'), ['demo'], ROOT / 'abi/rights.abi')
        for field in ('id', 'name', 'kind', 'capabilities'):
            previous = self.data['application'].copy()
            del self.data['application'][field]
            with self.assertRaisesRegex(ValueError, 'application.' + field):
                self.parse()
            self.data['application'] = previous

    def test_generation_safe_template_and_exclusive_destination(self):
        with tempfile.TemporaryDirectory() as directory:
            args = argparse.Namespace(path=Path(directory) / 'demo', id=None, name='Демо', kind='cli')
            create(args)
            app = load_manifest(args.path / 'dunit.toml', ['demo'], ROOT / 'abi/rights.abi')
            self.assertEqual(app.name, 'Демо')
            source = (args.path / 'src/main.rs').read_text()
            self.assertNotIn('unsafe', source)
            self.assertNotIn('extern "C"', source)
            self.assertIn('#[dunit::test]', source)
            with self.assertRaises(FileExistsError):
                create(args)
            args.path = Path(directory) / 'gui'
            args.kind = 'gui'
            with self.assertRaisesRegex(ValueError, 'window helper'):
                create(args)
            self.assertFalse(args.path.exists())
            args.kind = 'cli'
            args.name = '\n'
            with self.assertRaisesRegex(ValueError, 'application.name'):
                create(args)
            self.assertFalse(args.path.exists())


if __name__ == '__main__':
    unittest.main()
