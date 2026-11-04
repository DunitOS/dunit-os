"""Mutation checks for the production control-ABI source gate."""
from pathlib import Path
import tempfile
import unittest

import check_gui_protocol as gate


class ProtocolSourceTests(unittest.TestCase):
    def tree(self, root):
        canonical = root / gate.AUTHORITY
        canonical.parent.mkdir(parents=True)
        canonical.write_text((gate.ROOT / gate.AUTHORITY).read_text())
        return canonical

    def reject(self, relative, source):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.tree(root)
            client = root / relative
            client.parent.mkdir(parents=True, exist_ok=True)
            client.write_text(source)
            with self.assertRaisesRegex(ValueError, 'copied control constant'):
                gate.check(root)

    def test_repository_has_one_authoritative_codec(self):
        count, sources = gate.check()
        self.assertGreater(count, 0)
        self.assertGreater(sources, 0)

    def test_reintroduced_byte_tags_are_rejected_at_each_package_depth(self):
        for path in ('runtime/new/src/client.rs', 'userspace/system_apps/new/src/main.rs',
                     'userspace/libdunit/src/client.rs', 'protocols/gui-v1/src/copied.rs',
                     'kernel/src/copied.rs'):
            with self.subTest(path=path):
                self.reject(path, 'fn send() { let header = b"CAP1"; }')

    def test_numeric_and_raw_or_escaped_spellings_cannot_hide_a_copy(self):
        for literal in ('0x3150_4143u32', '827343171', bin(827343171), oct(827343171),
                        'br##"CAP1"##', 'b"\\x43AP1"'):
            with self.subTest(literal=literal):
                self.reject('runtime/new/src/lib.rs', 'const COPY: u32 = ' + literal + ';')

    def test_local_magic_redefinition_is_rejected_even_with_an_alias_value(self):
        self.reject('runtime/new/src/lib.rs', 'const INPUT_MAGIC: u32 = imported::INPUT_MAGIC;')

    def test_comments_diagnostic_strings_and_external_golden_tests_are_allowed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            canonical = self.tree(root)
            canonical.write_text(canonical.read_text() +
                                 '\n// pub const EXAMPLE_MAGIC: u32 = 123;\n')
            source = root / 'runtime/new/src/lib.rs'
            source.parent.mkdir(parents=True)
            source.write_text('''// b"CAP1" and 0x31504143 are historical examples.
/* outer /* b"INP1" */ const INPUT_MAGIC: u32 = 0; */
const MESSAGE: &str = "comment /* syntax */ and b\\"CAP1\\"";
const EXAMPLE: &str = r###"b"CAP1" and // comments"###;
use gui_protocol_v1::control::INPUT_MAGIC;
''')
            golden = root / 'protocols/gui-v1/tests/golden.rs'
            golden.parent.mkdir()
            golden.write_text('assert_eq!(packet, b"CAP1");')
            count, _ = gate.check(root)
            self.assertGreater(count, 0)

    def test_missing_or_duplicate_authority_fails_closed(self):
        for content in ('', 'pub const INPUT_MAGIC: u32 = 1;',
                        'pub const A_MAGIC: u32 = u32::from_le_bytes(*b"test");\n'
                        'pub const B_MAGIC: u32 = u32::from_le_bytes(*b"test");'):
            with self.subTest(content=content), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                authority = self.tree(root)
                authority.write_text(content)
                with self.assertRaises(ValueError):
                    gate.check(root)


if __name__ == '__main__':
    unittest.main()
