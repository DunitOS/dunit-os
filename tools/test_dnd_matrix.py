"""Coverage/schema checks for the real application matrix, without booting."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import dnd_matrix
from dunit_manifest import parse_manifest

class MatrixTests(unittest.TestCase):
    def test_all_ordered_application_pairs_are_explicit(self):
        data=dnd_matrix.load();self.assertEqual(len(data['cells']),len(data['applications'])**2)
    def mutate(self,change):
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'matrix.json';data=json.loads(dnd_matrix.MATRIX.read_text());change(data);path.write_text(json.dumps(data))
            with patch.object(dnd_matrix,'MATRIX',path):
                with self.assertRaises(ValueError):dnd_matrix.load()
    def test_new_application_without_matrix_entry_fails(self):
        with tempfile.TemporaryDirectory() as folder:
            registry=Path(folder)/'desktop.toml'
            registry.write_text(dnd_matrix.REGISTRY.read_text()+'\n[application.untracked]\nexec="/app/new_gui"\n')
            with patch.object(dnd_matrix,'REGISTRY',registry):
                with self.assertRaisesRegex(ValueError,'untracked'):dnd_matrix.load()
    def test_missing_cell_fails(self):self.mutate(lambda data:data['cells'].pop())
    def test_duplicate_cell_fails(self):self.mutate(lambda data:data['cells'].append(data['cells'][0]))
    def test_contradictory_expected_result_fails(self):self.mutate(lambda data:data['cells'][0].update(expected='text'))
    def test_keyboard_edges_are_balanced_without_virtual_release_timers(self):
        with tempfile.TemporaryDirectory() as folder:
            overlay=Path(folder)/'overlay';overlay.mkdir()
            with patch.dict('os.environ',{},clear=True):markers=dnd_matrix.prepare(overlay)
            actions=json.loads(markers.read_text())['run']['command_actions']['exec /app/gui_server']
            held=set()
            for action in actions:
                self.assertNotEqual(action.get('execute'),'send-key')
                for event in action.get('arguments',{}).get('events',[]):
                    if event['type']!='key':continue
                    data=event['data'];key=data['key']['data']
                    if data['down']:
                        self.assertNotIn(key,held);held.add(key)
                    else:
                        self.assertIn(key,held);held.remove(key)
            self.assertFalse(held)

    def test_native_aliases_launch_production_binaries_and_require_every_marker(self):
        with tempfile.TemporaryDirectory() as folder:
            overlay=Path(folder)/'overlay';overlay.mkdir()
            with patch.dict('os.environ',{},clear=True):markers=dnd_matrix.prepare(overlay)
            data=dnd_matrix.load();required={entry['marker'] for entry in json.loads(markers.read_text())['required']}
            for cell in data['cells']:self.assertIn(f"[DND] {cell['source']}->{cell['target']} ok",required)
            for role in ('s','t'):
                for app in data['applications']:
                    path=overlay/'system/share/dunit/apps'/f'm77_{role}_{app}'/'dunit.toml'
                    parsed=parse_manifest(path.read_text(),path,[app],dnd_matrix.ROOT/'abi/rights.abi')
                    self.assertEqual(parsed.binary,app)

if __name__=='__main__':unittest.main()
