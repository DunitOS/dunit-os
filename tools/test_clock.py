#!/usr/bin/env python3
"""Actual clock fallback, bounded ACPI parser and HPET conversion tests."""
import os
import subprocess
import tempfile
from pathlib import Path
from dunit_cli import ROOT

if __name__ == '__main__':
    with tempfile.TemporaryDirectory() as folder:
        binary=Path(folder)/'clock'
        subprocess.run([os.environ.get('RUSTC','rustc'),'--edition=2021','--test',str(ROOT/'tools/test_clock_host.rs'),'-o',str(binary)],check=True)
        subprocess.run([str(binary)],check=True,timeout=30)
