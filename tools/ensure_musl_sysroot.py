#!/usr/bin/env python3
"""Invalidate native libc artifacts whenever the actual fork inputs change."""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess


def fingerprint(source):
    names = subprocess.check_output(
        ['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard'], cwd=source
    ).split(b'\0')
    digest = hashlib.sha256()
    for name in sorted(set(names)):
        if not name:
            continue
        path = source / os.fsdecode(name)
        if path.is_file():
            digest.update(name + b'\0' + path.read_bytes())
    compiler = str(Path(shutil.which(os.environ.get('CC', 'cc')) or os.environ.get('CC', 'cc')).resolve())
    digest.update(compiler.encode())
    for key in ('CFLAGS', 'CPPFLAGS', 'LDFLAGS', 'AR', 'RANLIB'):
        digest.update(key.encode() + b'=' + os.environ.get(key, '').encode() + b'\0')
    digest.update(subprocess.check_output([compiler, '--version']))
    return digest.hexdigest()


def install_headers(root, sysroot):
    """Publish common native headers independently of libc's source cache."""
    source = root / 'abi/include'
    for header in sorted(source.rglob('*.h')):
        target = sysroot / 'include' / header.relative_to(source)
        data = header.read_bytes()
        if not target.is_file() or target.read_bytes() != data:
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)


def main():
    root = Path(__file__).resolve().parent.parent
    source = root / 'toolchains/dunit-musl'
    sysroot = root / 'build/dunit-sysroot'
    build = root / 'build/dunit-musl-build'
    stamp = sysroot / '.dunit-source-fingerprint'
    wanted = fingerprint(source)
    if (sysroot / 'lib/libc.a').is_file() and stamp.is_file() and stamp.read_text().strip() == wanted:
        install_headers(root, sysroot)
        return
    print('[MUSL] source/compiler changed: rebuilding native sysroot', flush=True)
    for path in (sysroot, build):
        if path.is_symlink():
            raise RuntimeError(f'refusing generated-directory symlink: {path}')
        if path.exists():
            shutil.rmtree(path)
    subprocess.run([str(source / 'tools/dunit/build-libc.sh'), str(sysroot)], cwd=root, check=True)
    # Record the inputs this build started with. If sources change during the
    # build, the next invocation must detect that change instead of caching it.
    stamp.write_text(wanted + '\n')
    install_headers(root, sysroot)


if __name__ == '__main__':
    main()
