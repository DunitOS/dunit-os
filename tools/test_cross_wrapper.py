#!/usr/bin/env python3
"""Exercise the actual installed-wrapper source without building target code."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parents[1] / 'toolchains/dunit-musl/tools/dunit/build-libc.sh'

class WrapperTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / 'sys root'
        (self.root / 'bin').mkdir(parents=True)
        source = SOURCE.read_text().split("<<'WRAP'\n", 1)[1].split('\nWRAP', 1)[0]
        self.wrapper = self.root / 'bin/dunit-cc'
        self.wrapper.write_text(source)
        self.wrapper.chmod(0o755)
        self.record = Path(self.temp.name) / 'record'
        self.compiler = Path(self.temp.name) / 'fake-cc'
        self.compiler.write_text('#!/usr/bin/python3\nimport os,sys,json\n'
            'if sys.argv[1:]==["-print-libgcc-file-name"]: print("/target/libgcc.a")\n'
            'else: open(os.environ["RECORD"],"w").write(json.dumps({"args":sys.argv[1:],"CPATH":os.environ.get("CPATH"),"LIBRARY_PATH":os.environ.get("LIBRARY_PATH")}))\n')
        self.compiler.chmod(0o755)
        self.env = dict(os.environ, CC=str(self.compiler), RECORD=str(self.record),
                        CPATH='/usr/include', LIBRARY_PATH='/usr/lib')

    def invoke(self, *args):
        return subprocess.run([str(self.wrapper), *args], env=self.env, capture_output=True, text=True)

    def test_compile_uses_target_headers_without_host_environment(self):
        self.assertEqual(self.invoke('-c', 'source file.c').returncode, 0)
        record = json.loads(self.record.read_text())
        self.assertIsNone(record['CPATH'])
        self.assertIsNone(record['LIBRARY_PATH'])
        self.assertEqual(record['args'][1:3], ['-isystem', str(self.root / 'include')])
        self.assertIn('source file.c', record['args'])
        self.assertNotIn('-nostdlib', record['args'])

    def test_link_uses_only_explicit_target_start_and_library_inputs(self):
        self.assertEqual(self.invoke('source.c', '-o', 'app').returncode, 0)
        args = json.loads(self.record.read_text())['args']
        for name in ('crt1.o', 'crti.o', 'libc.a', 'crtn.o'):
            self.assertIn(str(self.root / 'lib' / name), args)
        self.assertIn('-nostdlib', args)
        self.assertIn('/target/libgcc.a', args)

    def test_explicit_host_headers_rejected_before_compiler(self):
        for flags in [('-I/usr/include',), ('-I', '/usr/include/x86_64-linux-gnu'),
                      ('-isystem/usr/local/include',), ('-isystem', '/usr/include')]:
            with self.subTest(flags=flags):
                result = self.invoke(*flags, '-c', 'file.c')
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('host include directory', result.stderr)
                self.assertFalse(self.record.exists())

if __name__ == '__main__': unittest.main()
