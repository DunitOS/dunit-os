"""Private network smoke activation uses exact, bounded boot options."""
import ctypes
from pathlib import Path
import subprocess
import tempfile
import unittest

from dunit_cli import ROOT
from qemu_test import network_test_config


class NetworkConfiguration(unittest.TestCase):
    def test_preserves_smoke_identity_and_original_configuration(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'limine_test_terminal.conf'
            text = '/test\n    cmdline: mode=terminal other=1\n'
            source.write_text(text)
            result = Path(network_test_config(str(source), root / 'run'))
            self.assertEqual(result.name, source.name)
            self.assertEqual(source.read_text(), text)
            self.assertIn('cmdline: mode=terminal other=1 net_irq_test=1', result.read_text())
            result = Path(network_test_config(str(result), root / 'run'))
            self.assertEqual(result.read_text().count('net_irq_test=1'), 1)

    def test_rejects_production_ambiguous_and_conflicting_activation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, text in [
                ('limine.conf', 'cmdline: mode=terminal\n'),
                ('limine_test_terminal.conf', 'protocol: limine\n'),
                ('limine_test_terminal.conf', 'cmdline: a\ncmdline: b\n'),
                ('limine_test_terminal.conf', 'cmdline: net_irq_test=0\n'),
            ]:
                source = root / name
                source.write_text(text)
                with self.assertRaises(ValueError):
                    network_test_config(str(source), root / 'run')

    def test_actual_hal_parser_requires_an_exact_bounded_token(self):
        source = (ROOT / 'hal/src/boot_main.c').read_text()
        start = source.index('static int cmdline_has_token(')
        end = source.index('\n\nextern void kernel_main(', start)
        actual = source[start:end]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            c = root / 'options.c'
            c.write_text(actual + '\nint check(const char *line) {\n'
                         'return cmdline_has_token(line,"net_irq_test=1"); }\n')
            library = root / 'options.so'
            subprocess.run(['cc', '-shared', '-fPIC', '-Wall', '-Wextra', '-Werror',
                            str(c), '-o', str(library)], check=True)
            check = ctypes.CDLL(str(library)).check
            check.argtypes = [ctypes.c_char_p]
            for text in [b'net_irq_test=1', b'mode=terminal net_irq_test=1 other=0',
                         b'\t net_irq_test=1\t']:
                self.assertEqual(check(text), 1, text)
            for text in [None, b'', b'no_net_irq_test=1', b'net_irq_test=10',
                         b'net_irq_test=1suffix', b'x=net_irq_test=1', b'x' * 4096,
                         b'x' * 4090 + b' net_irq_test=1']:
                self.assertEqual(check(text), 0, text)


if __name__ == '__main__':
    unittest.main()
