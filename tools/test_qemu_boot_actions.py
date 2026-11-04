"""Boot-scene actions honor their deadlines independently of command budgets."""
import contextlib
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch
import qemu_test


class BootActionBudgetTests(unittest.TestCase):
    def scenario(self, *, late=20, step_timeout=30, overall=60, command=False):
        clock = SimpleNamespace(now=0.0)
        def advance(seconds):
            clock.now += seconds
        serial = Mock()
        serial.text = 'ready\n'
        def pump():
            if clock.now >= late and 'late\n' not in serial.text:
                serial.text += 'late\n'
        serial.pump.side_effect = pump
        serial.wait_for_marker.return_value = True
        actions = [{'wait_marker': 'late', 'timeout': step_timeout}]
        defaults = ({'command_actions': {'exec sample': actions}} if command else {'actions': actions})
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            image = root / 'image.iso'; image.touch()
            markers = root / 'markers.json'
            markers.write_text(json.dumps({'run': defaults, 'required': ['ready', 'late'], 'forbidden': ['PANIC']}))
            args = SimpleNamespace(workdir=folder, markers_file=str(markers), require_marker=[],
                forbid_marker=[], config=qemu_test.DEFAULT_CONFIG, build=False, build_disk=False,
                disk=None, iso=str(image), cmd=['exec sample'] if command else [], scratch_disk=None,
                scratch_bus='ahci', uefi=False, qemu_arg=[], accel='tcg', mem='64M', interactive=False,
                timeout=overall, boot_timeout=20, ready_marker='ready', post_boot_delay=0,
                cmd_timeout=15, settle=0, linger=0, screenshot=None)
            process = Mock(returncode=0)
            with patch.object(qemu_test, 'Qmp'), patch.object(qemu_test, 'SerialTail', return_value=serial), \
                 patch.object(qemu_test, 'build_qemu_command', return_value=['fake-qemu']), \
                 patch.object(qemu_test.subprocess, 'Popen', return_value=process), \
                 patch.object(qemu_test, 'type_command'), \
                 patch.object(qemu_test.time, 'monotonic', side_effect=lambda: clock.now), \
                 patch.object(qemu_test.time, 'sleep', side_effect=advance), \
                 contextlib.redirect_stderr(io.StringIO()):
                result = qemu_test.run(args)
            self.assertTrue(process.wait.called)
        return result, clock.now

    def test_boot_marker_after_default_command_budget_still_passes(self):
        result, elapsed = self.scenario()
        self.assertTrue(result.ok, result.reason)
        self.assertGreaterEqual(elapsed, 20)

    def test_individual_marker_timeout_still_fails(self):
        result, elapsed = self.scenario(step_timeout=5)
        self.assertFalse(result.ok)
        self.assertLess(elapsed, 6)

    def test_overall_run_deadline_still_caps_boot_actions(self):
        result, elapsed = self.scenario(late=40, step_timeout=50, overall=25)
        self.assertFalse(result.ok)
        self.assertLess(elapsed, 26)

    def test_command_actions_keep_their_command_budget(self):
        result, elapsed = self.scenario(command=True)
        self.assertFalse(result.ok)
        self.assertLess(elapsed, 16)


if __name__ == '__main__':
    unittest.main()
