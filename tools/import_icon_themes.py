#!/usr/bin/env python3
"""Reproduce complete, deterministic icon packages from a pinned upstream tree."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import zipfile
import zlib


def git(source, *args):
    return subprocess.check_output(['git', '-C', str(source), *args])


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--source', required=True, type=Path)
    ap.add_argument('--destination', required=True, type=Path)
    args = ap.parse_args()
    config = json.loads((args.destination / 'sources.json').read_text())
    revision = git(args.source, 'rev-parse', 'HEAD').decode().strip()
    if revision != config['revision']:
        raise SystemExit('source does not match pinned revision')
    if git(args.source, 'status', '--porcelain', '--untracked-files=no'):
        raise SystemExit('upstream tracked files must be unchanged')
    entries = []
    for record in git(args.source, 'ls-files', '--stage', '-z').split(b'\0'):
        if not record:
            continue
        meta, name = record.split(b'\t', 1)
        mode = int(meta.split()[0], 8)
        name = name.decode('utf-8')
        path = Path(name)
        if mode not in (0o100644, 0o100755) or path.is_absolute() or '..' in path.parts:
            raise SystemExit('unsupported upstream entry: ' + name)
        entries.append((name, mode))
    covered = {'AUTHORS', 'LICENSE', 'README.md'}
    for theme in config['themes']:
        covered.update(name for name, _ in entries if name.startswith(theme['directory'] + '/'))
    if covered != {name for name, _ in entries}:
        raise SystemExit('catalog does not cover the complete upstream tree')
    packages = []
    for theme in config['themes']:
        prefix = theme['directory'] + '/'
        selected = [(name, mode) for name, mode in entries
                    if name.startswith(prefix) or name in ('AUTHORS', 'LICENSE', 'README.md')]
        if not any(name == prefix + 'index.theme' for name, _ in selected):
            raise SystemExit('theme index missing: ' + prefix)
        output = args.destination / (theme['id'] + '.zip')
        temporary = output.with_suffix('.zip.tmp')
        with zipfile.ZipFile(temporary, 'w', compression=zipfile.ZIP_DEFLATED,
                             compresslevel=9) as archive:
            for name, mode in selected:
                info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
                info.create_system = 3
                info.external_attr = mode << 16
                info.compress_type = zipfile.ZIP_DEFLATED
                archive.writestr(info, (args.source / name).read_bytes(), compresslevel=9)
        with zipfile.ZipFile(temporary) as archive:
            if archive.testzip() is not None:
                raise SystemExit('archive CRC validation failed')
            for item in archive.infolist():
                # Compare the uncompressed bytes of every single upstream entry.
                if archive.read(item) != (args.source / item.filename).read_bytes():
                    raise SystemExit('archive changed source bytes: ' + item.filename)
        temporary.replace(output)
        original = [(name, mode) for name, mode in selected if name.startswith(prefix)]
        packages.append(dict(theme, archive=output.name, entries=len(selected),
                             files=len(original), svg=sum(n.endswith('.svg') for n, _ in original),
                             unpacked_bytes=sum((args.source / n).stat().st_size for n, _ in original),
                             archive_bytes=output.stat().st_size,
                             sha256=hashlib.sha256(output.read_bytes()).hexdigest(),
                             crc32=f'{zlib.crc32(output.read_bytes()):08x}'))
        print(output.name, len(original), 'files;', output.stat().st_size, 'bytes')
    for name in ('AUTHORS', 'LICENSE'):
        shutil.copyfile(args.source / name, args.destination / name)
    shutil.copyfile(args.source / 'README.md', args.destination / 'UPSTREAM-README.md')
    catalog = dict(repository=config['repository'], revision=revision,
                   packaging='Complete upstream theme trees in standard ZIP/DEFLATE; original bytes and paths.',
                   themes=packages)
    (args.destination / 'catalog.json').write_text(json.dumps(catalog, indent=2) + '\n')
    (args.destination / 'catalog.tsv').write_text(''.join(
        f"{p['archive']}\t{p['entries']}\t{p['archive_bytes']}\t{p['sha256']}\t{p['crc32']}\n"
        for p in packages))


if __name__ == '__main__':
    main()
