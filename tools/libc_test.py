#!/usr/bin/env python3
"""Verify and build a pinned, unmodified libc-test subset for native Dunit."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / 'tests/libc'
MANIFEST = BASE / 'manifest.json'


def binary_name(path):
    return 'libc_test_' + path.removeprefix('src/').removesuffix('.c').replace('/', '_')


def selected(manifest):
    return [case for case in manifest['cases'] if case['status'] == 'included']


def verify(manifest, base=BASE):
    if manifest['schema'] != 1 or not re.fullmatch(r'[0-9a-f]{40}', manifest['upstream']['commit']):
        raise ValueError('invalid manifest schema or commit')
    inventory = manifest['cases']
    paths = [entry['path'] for entry in inventory]
    if len(paths) != len(set(paths)) or not selected(manifest):
        raise ValueError('duplicate inventory paths or empty subset')
    vendored = {entry['path']: entry['sha256'] for entry in manifest['vendored_files']}
    for case in inventory:
        if case['status'] not in {'included', 'deferred', 'support'} or not case['reason'] or not case['owner']:
            raise ValueError('case missing status/reason/owner: ' + case['path'])
        if case['status'] in {'included', 'support'} and vendored.get(case['path']) != case['sha256']:
            raise ValueError('case is not pinned to its vendored source: ' + case['path'])
    actual = {p.relative_to(base / 'upstream').as_posix() for p in (base / 'upstream').rglob('*') if p.is_file()}
    if actual != set(vendored):
        raise ValueError('vendored source inventory mismatch')
    for path, expected in vendored.items():
        if Path(path).is_absolute() or '..' in Path(path).parts:
            raise ValueError('unsafe source path')
        got = hashlib.sha256((base / 'upstream' / path).read_bytes()).hexdigest()
        if got != expected:
            raise ValueError('upstream source was modified: ' + path)


def verify_upstream(manifest, checkout):
    commit = subprocess.check_output(['git', '-C', str(checkout), 'rev-parse', 'HEAD'], text=True).strip()
    if commit != manifest['upstream']['commit']:
        raise ValueError('upstream checkout is not the pinned commit')
    paths = {p.relative_to(checkout).as_posix() for p in checkout.glob('src/*/*.c')}
    if paths != {case['path'] for case in manifest['cases']}:
        raise ValueError('upstream case inventory incomplete')
    entries = manifest['cases'] + manifest['vendored_files']
    for entry in entries:
        got = hashlib.sha256((checkout / entry['path']).read_bytes()).hexdigest()
        if got != entry['sha256']:
            raise ValueError('pinned upstream mismatch: ' + entry['path'])


def marker_document(manifest):
    cases = selected(manifest)
    return {
        'description': 'Pinned upstream libc-test subset; no modified assertions. Requires every case return success and no upstream t_error diagnostics.',
        'run': {'config': 'limine_test_terminal.conf', 'cmd': ['exec /app/' + binary_name(case['path']) for case in cases]},
        'required': [{'marker': '[TERM-007] Starting main loop', 'claim': 'terminal boot reached', 'source': 'kernel/src/lib.rs'}] + [
            {'marker': '[LIBC-TEST] ' + case['path'][4:-2] + ' PASS rc=0 status=0',
             'claim': 'Unmodified original case returned zero and upstream t_status remained zero',
             'source': 'tests/libc/upstream/' + case['path']} for case in cases],
        'forbidden': [{'marker': '[PANIC]', 'why': 'kernel panic', 'source': 'kernel/src/lib.rs'},
                      {'marker': ' FAIL rc=', 'why': 'upstream case or t_status failed', 'source': 'tests/libc/runner.c'}],
    }


def build(manifest, cc, output):
    output.mkdir(parents=True, exist_ok=True)
    source = BASE / 'upstream'
    # Keep object intermediates outside the flat binary directory packed by Makefile.
    with tempfile.TemporaryDirectory(prefix='dunit-libc-test-') as scratch:
        scratch = Path(scratch)
        flags = ['-O2', '-fno-builtin', '-I' + str(source / 'src/common')]
        support = []
        for name in ('print', 'rand'):
            obj = scratch / (name + '.o')
            subprocess.run([cc, *flags, '-c', str(source / 'src/common' / (name + '.c')), '-o', str(obj)], check=True)
            support.append(str(obj))
        for case in selected(manifest):
            name = binary_name(case['path'])
            obj = scratch / (name + '.o')
            subprocess.run([cc, *flags, '-Dmain=libc_test_case_main', '-c', str(source / case['path']), '-o', str(obj)], check=True)
            subprocess.run([cc, *flags, '-DDUNIT_LIBC_TEST_NAME="' + case['path'][4:-2] + '"',
                            str(BASE / 'runner.c'), str(obj), *support, '-lm', '-o', str(output / name)], check=True)
            print('[LIBC-TEST-BUILD]', name, flush=True)
    # Build success is deliberately not reported as native test success.


class OrchestrationTests(unittest.TestCase):
    def test_pins_and_complete_local_inventory(self):
        verify(json.loads(MANIFEST.read_text()))

    def test_markers_match_every_selected_case(self):
        manifest = json.loads(MANIFEST.read_text())
        self.assertEqual(json.loads((ROOT / 'tools/libc_test_markers.json').read_text()), marker_document(manifest))

    def test_bad_pinned_content_is_rejected(self):
        manifest = json.loads(MANIFEST.read_text())
        manifest['vendored_files'][0]['sha256'] = '0' * 64
        with self.assertRaises(ValueError):
            verify(manifest)

    def test_missing_reason_is_rejected(self):
        manifest = json.loads(MANIFEST.read_text())
        manifest['cases'][0]['reason'] = ''
        with self.assertRaises(ValueError):
            verify(manifest)

    def test_build_keeps_original_cases_in_separate_translation_units(self):
        manifest = json.loads(MANIFEST.read_text())
        calls = []
        def compiler(command, check):
            self.assertTrue(check)
            self.assertEqual(command[0], 'native-cc')
            Path(command[command.index('-o') + 1]).write_bytes(b'object-placeholder')
            calls.append(command)
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            with mock.patch.object(subprocess, 'run', side_effect=compiler), mock.patch('builtins.print'):
                build(manifest, 'native-cc', output)
            self.assertEqual({p.name for p in output.iterdir()},
                             {binary_name(case['path']) for case in selected(manifest)})
        originals = [call for call in calls if '-Dmain=libc_test_case_main' in call]
        self.assertEqual(len(originals), len(selected(manifest)))
        self.assertTrue(all('-c' in call and '-fno-builtin' in call for call in originals))
        self.assertTrue(all(str(BASE / 'runner.c') not in call for call in originals))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    check = sub.add_parser('verify')
    check.add_argument('--upstream', type=Path)
    sub.add_parser('list')
    sub.add_parser('self-test')
    compile_parser = sub.add_parser('build')
    compile_parser.add_argument('--cc', required=True)
    compile_parser.add_argument('--out-dir', required=True, type=Path)
    args = parser.parse_args()
    if args.command == 'self-test':
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(OrchestrationTests)
        return 0 if unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful() else 1
    manifest = json.loads(MANIFEST.read_text())
    verify(manifest)
    if args.command == 'verify':
        if args.upstream:
            verify_upstream(manifest, args.upstream)
        print('libc-test: pins verified; included=%d inventory=%d' % (len(selected(manifest)), len(manifest['cases'])))
    elif args.command == 'list':
        print('\n'.join(binary_name(case['path']) for case in selected(manifest)))
    elif args.command == 'build':
        build(manifest, args.cc, args.out_dir)
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print('libc-test:', error, file=sys.stderr)
        sys.exit(1)
