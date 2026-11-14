#!/usr/bin/env python3
"""Actual IPC guard, FIFO and authenticated packet identity with host IRQ shims."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from dunit_cli import ROOT

class IpcLock(unittest.TestCase):
    def compile(self,negative=False):
        source=(ROOT/'tools/test_ipc_lock_host.rs.in').read_text()
        for key,path in [('SYNC','kernel/src/sync.rs'),('IPC','kernel/src/ipc/mod.rs')]:
            source=source.replace('@'+key+'@',json.dumps(str(ROOT/path)))
        source=source.replace('@NEGATIVE@',"let _escaped: &'static mut ipc::IpcManager = &mut *ipc::get_ipc_manager().unwrap();" if negative else '')
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'ipc.rs';path.write_text(source)
            result=subprocess.run([os.environ.get('RUSTC','rustc'),'--edition=2021',str(path),'-O','-o',str(path.with_suffix(''))],capture_output=True,text=True)
            if not negative and result.returncode==0:subprocess.run([str(path.with_suffix(''))],check=True,timeout=30)
            return result
    def test_concurrent_fifo_ownership_and_attachment_identity(self):
        result=self.compile();self.assertEqual(result.returncode,0,result.stderr)
    def test_static_manager_reference_cannot_escape_guard(self):
        result=self.compile(negative=True);self.assertNotEqual(result.returncode,0);self.assertIn('temporary value dropped while borrowed',result.stderr)
if __name__=='__main__':unittest.main()
