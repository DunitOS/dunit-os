"""Phase budgets reject guest faults without confusing intentional boot faults."""
import json
from pathlib import Path
import tempfile
import unittest
from qemu_test import forbidden_marker_hits, load_markers

class MarkerPhases(unittest.TestCase):
    def test_global_markers_still_cover_the_complete_log(self):
        self.assertEqual(forbidden_marker_hits('PANIC boot\nready\n', ['PANIC']), ['PANIC'])
    def test_phase_preserves_intentional_earlier_fault_but_rejects_later_fault(self):
        markers = [{'marker': 'FAULT', 'after': 'ready'}]
        self.assertEqual(forbidden_marker_hits('FAULT expected\nready\nsuccess', markers), [])
        self.assertEqual(forbidden_marker_hits('FAULT expected\nready\nFAULT guest', markers), ['FAULT'])
    def test_missing_phase_is_a_required_marker_failure(self):
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'markers.json'
            path.write_text(json.dumps({'forbidden': [{'marker': 'FAULT', 'after': 'ready'}]}))
            required, forbidden, _ = load_markers(str(path), None, None)
            self.assertEqual(required, ['ready'])
            self.assertEqual(forbidden_marker_hits('boot only', forbidden), [])
            self.assertEqual([m for m in required if m not in 'boot only'], ['ready'])
    def test_existing_annotated_markers_and_cli_remain_global(self):
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'markers.json'
            path.write_text(json.dumps({'required': ['ready'], 'forbidden': [{'marker': 'PANIC', 'why': 'global'}]}))
            required, forbidden, _ = load_markers(str(path), ['done'], ['FAULT'])
            self.assertEqual(required, ['ready', 'done'])
            self.assertEqual(forbidden, ['PANIC', 'FAULT'])
    def test_empty_boundary_is_invalid(self):
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'markers.json'
            path.write_text(json.dumps({'forbidden': [{'marker': 'FAULT', 'after': ''}]}))
            with self.assertRaises(ValueError): load_markers(str(path), None, None)

if __name__ == '__main__': unittest.main()
