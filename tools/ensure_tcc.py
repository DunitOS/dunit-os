#!/usr/bin/env python3
"""Invalidate the host Dunit cross-TCC when source/compiler/sysroot changes."""
import argparse
import hashlib
import os
from pathlib import Path
import subprocess
from ensure_musl_sysroot import fingerprint

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sysroot', type=Path, required=True)
    args=parser.parse_args()
    root=Path(__file__).resolve().parents[1]
    source=root/'toolchains/dunit-tcc'
    sysroot=args.sysroot.resolve()
    stamp=root/'build/.dunit-tcc-fingerprint'
    digest=hashlib.sha256(fingerprint(source).encode())
    digest.update((sysroot/'lib/libc.a').read_bytes())
    digest.update(str(sysroot).encode())
    wanted=digest.hexdigest()
    if (source/'tcc').is_file() and stamp.is_file() and stamp.read_text().strip()==wanted:
        return
    print('[TCC] source/compiler/sysroot changed: rebuilding cross compiler',flush=True)
    subprocess.run([str(source/'tools/dunit/build-tcc.sh'),str(sysroot)],cwd=root,check=True)
    stamp.write_text(wanted+'\n')

if __name__=='__main__': main()
