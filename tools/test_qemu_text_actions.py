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

if __name__ == '__main__':
    unittest.main()
