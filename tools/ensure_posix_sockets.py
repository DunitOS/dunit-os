#!/usr/bin/env python3
"""Build the opt-in POSIX socket archive against an existing Dunit musl sysroot."""
import argparse
import filecmp
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'runtime/dunit-posix-sockets'


def install(sysroot):
    sysroot = sysroot.resolve(strict=True)
    lib = sysroot / 'lib'
    header = sysroot / 'include/dunit/posix-sockets.h'
    header.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='posix-sockets-', dir=lib) as directory:
        folder = Path(directory)
        object_file = folder / 'socket.o'
        archive = folder / 'libdunit_posix_sockets.a'
        subprocess.run([
            sysroot / 'bin/dunit-cc', '-std=c11', '-O2', '-Wall', '-Wextra', '-Werror',
            '-I', SOURCE / 'include', '-c', SOURCE / 'socket.c', '-o', object_file,
        ], check=True)
        subprocess.run([sysroot / 'bin/x86_64-dunit-ar', 'rcs', archive, object_file], check=True)
        destination = lib / archive.name
        if not destination.is_file() or not filecmp.cmp(archive, destination, shallow=False):
            archive.replace(destination)
    source_header = SOURCE / 'include/dunit/posix-sockets.h'
    if not header.is_file() or not filecmp.cmp(source_header, header, shallow=False):
        shutil.copyfile(source_header, header)
    print(f'[POSIX-SOCKETS] installed {destination.name} ({destination.stat().st_size} bytes)')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sysroot', type=Path, required=True)
    install(parser.parse_args().sysroot)
