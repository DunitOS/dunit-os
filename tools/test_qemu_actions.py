import time
import unittest
import tempfile
from pathlib import Path
from unittest.mock import patch
import qemu_test
from qemu_test import run_command_actions
class Serial:
    def __init__(self,text):self.text=text
    def pump(self):pass
class MarkerWaitTests(unittest.TestCase):
    def test_wait_requires_the_requested_occurrence_within_current_command(self):
        run_command_actions(None,Serial('old ready new ready ready'),[{'wait_marker':'ready','occurrences':2}],10,time.monotonic()+1)
        with self.assertRaises(RuntimeError):run_command_actions(None,Serial('ready'),[{'wait_marker':'ready','occurrences':2,'timeout':0}],0,time.monotonic()+1)
    def test_default_remains_one_and_invalid_counts_are_rejected(self):
        run_command_actions(None,Serial('ready'),[{'wait_marker':'ready'}],0,time.monotonic()+1)
        for count in [0,-1,1025,'2']:
            with self.assertRaises(ValueError):run_command_actions(None,Serial('ready'),[{'wait_marker':'ready','occurrences':count}],0,time.monotonic()+1)
class DeveloperImageBuildTests(unittest.TestCase):
    def test_profile_and_overlay_reach_the_existing_make_build(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);(root/'build').mkdir();image=root/'build/microkernel.iso';image.touch();overlay=root/'overlay';overlay.mkdir()
            with patch.object(qemu_test,'REPO_ROOT',root),patch.object(qemu_test.shutil,'which',return_value='/usr/bin/make'),patch.object(qemu_test.subprocess,'run') as run:
                self.assertEqual(qemu_test.build_iso('limine_sdk_terminal.conf',overlay=overlay,profile='dwm'),image)
                arguments=run.call_args.args[0]
                self.assertIn('IMAGE_PROFILE=dwm',arguments);self.assertIn('LIMINE_CONFIG=limine_sdk_terminal.conf',arguments)
                self.assertIn('INITRD_OVERLAY='+str(overlay.resolve()),arguments)
    def test_release_variant_keeps_its_declared_profile(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);(root/'build').mkdir();(root/'build/dunit_minimal.iso').touch()
            with patch.object(qemu_test,'REPO_ROOT',root),patch.object(qemu_test.shutil,'which',return_value='/usr/bin/make'),patch.object(qemu_test.subprocess,'run') as run:
                qemu_test.build_iso('ignored.conf',variant='minimal')
                self.assertEqual(run.call_args.args[0],['make','iso-minimal'])

if __name__=='__main__':unittest.main()
