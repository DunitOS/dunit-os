"""Literal GUI text and terminal-submit contracts of the shared QEMU harness."""
import unittest
from unittest.mock import patch
import qemu_test

class Keyboard:
    def __init__(self):
        self.keys = []
    def send_key(self, keys):
        self.keys.append(keys)

class TextActions(unittest.TestCase):
    @patch('qemu_test.time.sleep')
    def test_terminal_typing_still_submits_by_default(self, sleep):
        keyboard = Keyboard()
        qemu_test.type_command(keyboard, 'ab')
        self.assertEqual(keyboard.keys, [['a'], ['b'], ['ret']])
    @patch('qemu_test.time.sleep')
    def test_literal_multiline_does_not_append_an_unrequested_enter(self, sleep):
        keyboard = Keyboard()
        qemu_test.type_command(keyboard, 'a\nB', submit=False)
        self.assertEqual(keyboard.keys, [['a'], ['ret'], ['shift', 'b']])
    @patch('qemu_test.time.sleep')
    def test_empty_literal_does_not_modify_the_focused_field(self, sleep):
        keyboard = Keyboard()
        qemu_test.type_command(keyboard, '', submit=False)
        self.assertEqual(keyboard.keys, [])
    @patch('qemu_test.time.sleep')
    def test_unsupported_text_is_rejected_before_any_partial_input_or_submit(self, sleep):
        for character in ('É', 'Я', 'я', '🙂', '\t'):
            with self.subTest(character=character):
                keyboard = Keyboard()
                with self.assertRaisesRegex(ValueError, 'guest US keyboard layout'):
                    qemu_test.type_command(keyboard, 'before' + character + 'after')
                self.assertEqual(keyboard.keys, [])
                self.assertIsNone(qemu_test.char_to_keys(character))

if __name__ == '__main__':
    unittest.main()
