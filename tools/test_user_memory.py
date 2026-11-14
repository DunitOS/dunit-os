#!/usr/bin/env python3
"""Actual user access/copy scopes with owned host mappings and privileged shims."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from dunit_cli import ROOT

class UserMemory(unittest.TestCase):
    def compile(self,negative=''):
        source=(ROOT/'tools/test_user_memory_host.rs.in').read_text()
        for name,path in [('SYNC','kernel/src/sync.rs'),('MEMORY','kernel/src/syscall/user_memory.rs')]:
            source=source.replace('@'+name+'@',json.dumps(str(ROOT/path)))
        source=source.replace('@NEGATIVE@',negative)
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'memory.rs';path.write_text(source)
            result=subprocess.run([os.environ.get('RUSTC','rustc'),'--edition=2021',str(path),'-O','-o',str(path.with_suffix(''))],capture_output=True,text=True)
            if not negative and result.returncode==0:subprocess.run([str(path.with_suffix(''))],check=True,timeout=30)
            return result
    def test_actual_mapping_pin_permissions_and_copies(self):
        result=self.compile();self.assertEqual(result.returncode,0,result.stderr)
    def test_borrowed_user_view_cannot_escape(self):
        result=self.compile('let _=with_mapping(ptr as u64,4,false,|mapping|Ok(mapping));');self.assertNotEqual(result.returncode,0);self.assertIn('lifetime',result.stderr)
    def test_user_view_cannot_migrate(self):
        result=self.compile('let _=with_mapping(ptr as u64,4,false,|mapping| {std::thread::spawn(move||mapping.read_byte(0));Ok(())});');self.assertNotEqual(result.returncode,0);self.assertIn('cannot be sent between threads',result.stderr)
if __name__=='__main__':unittest.main()
