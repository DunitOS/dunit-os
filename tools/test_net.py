#!/usr/bin/env python3
"""Actual E1000 descriptors, DMA lifetime and controller with host device transport."""
import os
from pathlib import Path
import subprocess
import tempfile
from dunit_cli import ROOT

if __name__ == '__main__':
    with tempfile.TemporaryDirectory() as folder:
        source = Path(folder) / 'net.rs'
        source.write_text((ROOT / 'tools/test_net_host.rs.in').read_text().replace('@ROOT@', str(ROOT)))
        binary = source.with_suffix('')
        subprocess.run([os.environ.get('RUSTC', 'rustc'), '--edition=2021', '--test', '-O', str(source), '-o', str(binary)], check=True)
        subprocess.run([str(binary), '--test-threads=1'], check=True, timeout=60)
        subprocess.run([os.environ.get('RUSTC', 'rustc'), '--edition=2021', '--test', '-O', str(ROOT / 'tools/test_net_discovery_host.rs'), '-o', str(binary)], check=True)
        subprocess.run([str(binary), '--test-threads=1'], check=True, timeout=30)
