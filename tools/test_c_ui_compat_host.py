#!/usr/bin/env python3
"""Regressions for frozen-client integrity and the link-only compatibility proof."""
import copy
import json
from pathlib import Path
import shutil
import struct
import tempfile
import unittest
from test_c_ui_compat import FIXTURE, ROOT, check_object, compatibility_contract, frozen_fixture

class Compatibility(unittest.TestCase):
    def test_frozen_inputs_match_the_published_v1_snapshot(self):
        manifest=frozen_fixture()
        self.assertEqual(manifest['source_commit'],'9df23f624c4bf805cca54653a1624d3e89488624')
    def test_changed_header_or_source_is_not_silently_refreshed(self):
        for name in ('gui.c','include/dunit/ui.h'):
            with self.subTest(name=name),tempfile.TemporaryDirectory()as folder:
                fixture=Path(folder)/'v1';shutil.copytree(FIXTURE,fixture)
                with (fixture/name).open('ab')as file:file.write(b'\n')
                with self.assertRaisesRegex(ValueError,'fixture changed'):frozen_fixture(fixture)
    def test_unexpected_header_override_is_rejected(self):
        with tempfile.TemporaryDirectory()as folder:
            fixture=Path(folder)/'v1';shutil.copytree(FIXTURE,fixture)
            (fixture/'include/stdio.h').write_text('/* stale or accidental override */')
            with self.assertRaisesRegex(ValueError,'include override'):frozen_fixture(fixture)
    def test_all_existing_behavior_and_pixel_actions_are_retained(self):
        original=json.loads((ROOT/'tools/c_ui_markers.json').read_text());before=copy.deepcopy(original)
        contract=compatibility_contract(original)
        self.assertEqual(original,before)
        self.assertEqual(contract['required'],original['required'])
        self.assertEqual(contract['forbidden'],original['forbidden'])
        self.assertEqual(contract['run']['command_actions'],original['run']['command_actions'])
        self.assertEqual(contract['run']['cmd'][1:],original['run']['cmd'][1:])
        self.assertIn('/c-ui-v1.o',contract['run']['cmd'][0])
        self.assertNotIn('.c ',contract['run']['cmd'][0])
    def test_guest_recompilation_invalidates_the_proof(self):
        original=json.loads((ROOT/'tools/c_ui_markers.json').read_text())
        original['run']['cmd'].append('exec /system/bin/tcc /new.c -o /tmp/new')
        with self.assertRaisesRegex(ValueError,'without C compilation'):compatibility_contract(original)
    def test_retained_object_must_stay_unchanged_and_relocatable(self):
        with tempfile.TemporaryDirectory()as folder:
            path=Path(folder)/'client.o';data=bytearray(64);data[:6]=b'\x7fELF\x02\x01';struct.pack_into('<HH',data,16,1,62)
            path.write_bytes(data);expected=check_object(path)
            path.write_bytes(data+b'changed')
            with self.assertRaisesRegex(ValueError,'changed after compilation'):check_object(path,expected)
            struct.pack_into('<H',data,16,2);path.write_bytes(data)
            with self.assertRaisesRegex(ValueError,'relocatable'):check_object(path)

if __name__=='__main__':unittest.main()
