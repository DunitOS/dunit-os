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
    def scenario(self, *, late=20, step_timeout=30, overall=60, command=False,
                 screenshot_error=None, wait_marker='late', action_screenshot=False,
                 log_error=None):
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
        actions = [{'wait_marker': wait_marker, 'timeout': step_timeout}]
        if action_screenshot:
            actions.append({'execute': 'screendump', 'arguments': {'filename': 'capture.ppm'}})
        defaults = ({'command_actions': {'exec sample': actions}} if command else {'actions': actions})
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            image = root / 'image.iso'; image.touch()
            markers = root / 'markers.json'
            markers.write_text(json.dumps({'run': defaults, 'required': ['ready', wait_marker], 'forbidden': ['PANIC']}))
            args = SimpleNamespace(workdir=folder, markers_file=str(markers), require_marker=[],
                forbid_marker=[], config=qemu_test.DEFAULT_CONFIG, build=False, build_disk=False,
                disk=None, iso=str(image), cmd=['exec sample'] if command else [], scratch_disk=None,
                scratch_bus='ahci', uefi=False, qemu_arg=[], accel='tcg', mem='64M', interactive=False,
                timeout=overall, boot_timeout=20, ready_marker='ready', post_boot_delay=0,
                cmd_timeout=15, settle=0, linger=0,
                screenshot='nested/capture.ppm' if screenshot_error and not action_screenshot else None)
            process = Mock(returncode=0)
            write_text = Path.write_text
            def persist(path, *args, **kwargs):
                if log_error and path.name == 'qemu-serial.full.log':
                    raise log_error
                return write_text(path, *args, **kwargs)
            with patch.object(qemu_test, 'Qmp') as qmp, patch.object(qemu_test, 'SerialTail', return_value=serial), \
                 patch.object(Path, 'write_text', autospec=True, side_effect=persist), \
                 patch.object(qemu_test, 'build_qemu_command', return_value=['fake-qemu']), \
                 patch.object(qemu_test.subprocess, 'Popen', return_value=process), \
                 patch.object(qemu_test, 'type_command'), \
                 patch.object(qemu_test.time, 'monotonic', side_effect=lambda: clock.now), \
                 patch.object(qemu_test.time, 'sleep', side_effect=advance), \
                 contextlib.redirect_stderr(io.StringIO()):
                qmp.return_value.screendump.side_effect = screenshot_error
                result = qemu_test.run(args)
            self.assertTrue(process.wait.called)
            if log_error is None:
                self.assertEqual(result.full_serial_log, str(root / 'qemu-serial.full.log'))
                self.assertEqual(Path(result.full_serial_log).read_text(), serial.text)
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

    def test_boot_actions_can_wait_for_the_already_observed_ready_marker(self):
        result, elapsed = self.scenario(wait_marker='ready')
        self.assertTrue(result.ok, result.reason)
        self.assertLess(elapsed, 1)

    def test_command_actions_cannot_reuse_the_boot_ready_marker(self):
        result, _ = self.scenario(command=True, wait_marker='ready')
        self.assertFalse(result.ok)

    def test_failed_requested_screenshot_fails_run_despite_good_markers(self):
        result, _ = self.scenario(screenshot_error=RuntimeError('capture rejected'))
        self.assertFalse(result.ok)
        self.assertEqual(result.missing_markers, [])
        self.assertIn('capture rejected', result.reason)
        self.assertIsNone(result.screenshot)

    def test_failed_action_screenshot_fails_run_despite_good_markers(self):
        result, _ = self.scenario(action_screenshot=True,
                                  screenshot_error=RuntimeError('capture missing'))
        self.assertFalse(result.ok)
        self.assertEqual(result.missing_markers, [])
        self.assertIn('capture missing', result.reason)

    def test_summary_reports_this_runs_persisted_log(self):
        result, _ = self.scenario()
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            qemu_test.print_human(result)
        self.assertIn(f'Serial log: {result.full_serial_log}', output.getvalue())
        self.assertNotIn('Serial log: build/qemu-serial.full.log', output.getvalue())

    def test_log_write_failure_fails_run_despite_good_markers(self):
        result, _ = self.scenario(log_error=OSError('disk full'))
        self.assertFalse(result.ok)
        self.assertEqual(result.missing_markers, [])
        self.assertIsNone(result.full_serial_log)
        self.assertIn('cannot save full serial log: disk full', result.reason)


if __name__ == '__main__':
    unittest.main()
