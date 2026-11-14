#!/usr/bin/env python3
"""Actual parking/registry/wait code under deterministic remote notifications."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from dunit_cli import ROOT


class Parking(unittest.TestCase):
    def test_remote_gap_stale_callback_and_deadlines(self):
        source=(ROOT/'tools/test_parking_host.rs.in').read_text()
        for name,path in [('SYNC','kernel/src/sync.rs'),('REGISTRY','kernel/src/process/registry.rs'),
                          ('WAIT','kernel/src/process/wait_queue.rs'),('PARKING','kernel/src/process/parking.rs')]:
            source=source.replace('@'+name+'@',json.dumps(str(ROOT/path)))
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'parking.rs';path.write_text(source)
            subprocess.run([os.environ.get('RUSTC','rustc'),'--edition=2021',str(path),'-O','-o',str(path.with_suffix(''))],check=True)
            subprocess.run([str(path.with_suffix(''))],check=True,timeout=30)


if __name__=='__main__':
    unittest.main()
