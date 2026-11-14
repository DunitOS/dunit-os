#!/usr/bin/env python3
"""Actual TLB request/generation/ack protocol with host platform transport."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
from dunit_cli import ROOT

if __name__=='__main__':
    source=(ROOT/'tools/test_tlb_host.rs.in').read_text()
    for name,path in [('SYNC','kernel/src/sync.rs'),('TLB','kernel/src/memory/tlb.rs')]:
        source=source.replace('@'+name+'@',json.dumps(str(ROOT/path)))
    with tempfile.TemporaryDirectory() as folder:
        path=Path(folder)/'tlb.rs';path.write_text(source)
        subprocess.run([os.environ.get('RUSTC','rustc'),'--edition=2021','--cfg','feature="boot-smoke-tests"',str(path),'-O','-o',str(path.with_suffix(''))],check=True)
        subprocess.run([str(path.with_suffix(''))],check=True,timeout=60)
