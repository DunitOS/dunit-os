#!/usr/bin/env python3
"""Create, verify and transactionally install a relocatable Dunit SDK (Python 3.11+)."""
import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import tarfile
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = 'sdk-manifest.json'
MAX_FILES = 50000
MAX_BYTES = 512 * 1024 * 1024


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def safe_name(name):
    path = PurePosixPath(name)
    if not name or path.is_absolute() or '\\' in name or any(ord(c) < 32 for c in name) or any(p in ('', '.', '..') for p in name.split('/')):
        raise ValueError(f'unsafe archive path: {name!r}')
    return path


def regular_files(root):
    for path in sorted(root.rglob('*')):
        if path.is_symlink() or (not path.is_dir() and not path.is_file()):
            raise ValueError(f'unsupported special file: {path}')
        if path.is_file():
            yield path


def copy_tree(source, destination):
    for path in sorted(source.rglob("*")):
        if path.is_symlink():
            path.resolve(strict=True).relative_to(source.resolve())
        if path.is_dir():
            continue
        if not path.is_file():
            raise ValueError(f"unsupported source file: {path}")
        relative = path.relative_to(source)
        if any(part in ('target', '.git', '__pycache__') for part in relative.parts):
            continue
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, target)
        target.chmod(0o755 if path.stat().st_mode & 0o111 else 0o644)


def crate_graph(repository):
    pending = [repository / 'runtime/dunit', repository / 'runtime/dunit-c']
    found = {}
    while pending:
        directory = pending.pop().resolve()
        directory.relative_to(repository)
        if directory in found:
            continue
        data = tomllib.loads((directory / 'Cargo.toml').read_text())
        found[directory] = data['package']
        def dependencies(table):
            for key, value in table.items():
                if key in ('dependencies', 'dev-dependencies', 'build-dependencies'):
                    for dependency in value.values():
                        if isinstance(dependency, dict) and 'path' in dependency:
                            pending.append(directory / dependency['path'])
                elif isinstance(value, dict):
                    dependencies(value)
        dependencies(data)
    return found


def vendor_crate(package, cargo_home, destination):
    name = f"{package['name']}-{package['version']}"
    archives = sorted((cargo_home / 'registry/cache').glob(f'*/{name}.crate'))
    if not archives:
        raise ValueError(f'missing cached registry crate {name}; fetch dependencies before packaging')
    archive = archives[0]
    if digest(archive) != package['checksum']:
        raise ValueError(f'registry checksum mismatch: {name}')
    output = destination / name
    output.mkdir()
    seen = set()
    total = 0
    with tarfile.open(archive, 'r:gz') as source:
        for member in source:
            path = safe_name(member.name)
            if path.parts[0] != name or not (member.isfile() or member.isdir()):
                raise ValueError(f'invalid registry member: {member.name}')
            if member.isdir():
                continue
            if member.name in seen:
                raise ValueError(f'duplicate registry member: {member.name}')
            seen.add(member.name)
            total += member.size
            if total > MAX_BYTES or len(seen) > MAX_FILES:
                raise ValueError('registry archive exceeds limits')
            target = output.joinpath(*path.parts[1:])
            target.parent.mkdir(parents=True, exist_ok=True)
            with source.extractfile(member) as stream, target.open('xb') as sink:
                shutil.copyfileobj(stream, sink)
            target.chmod(0o755 if member.mode & 0o111 else 0o644)
    checksums = {str(p.relative_to(output)): digest(p) for p in regular_files(output) if p.name != '.cargo-checksum.json'}
    (output / '.cargo-checksum.json').write_text(json.dumps({'package': package['checksum'], 'files': checksums}, sort_keys=True) + '\n')


def create(output, sysroot, cargo_home, rust_source, repository=ROOT):
    repository = repository.resolve()
    output = output.absolute()
    if output.exists():
        raise ValueError(f'output already exists: {output}')
    graph = crate_graph(repository)
    for required in ('bin/x86_64-dunit-cc', 'include/dunit/ui.h', 'lib/libc.a', 'lib/libdunit_ui.a', 'lib/crt1.o'):
        if not (sysroot / required).is_file():
            raise ValueError(f'incomplete prepared sysroot: {sysroot / required}')
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.sdk-bundle-', dir=output.parent) as temporary:
        staging = Path(temporary) / 'sdk'
        staging.mkdir()
        packages = {}
        local = {}
        for directory, package in sorted(graph.items()):
            relative = directory.relative_to(repository)
            copy_tree(directory, staging / relative)
            local[package['name']] = {'path': str(relative), 'version': package['version']}
            lock = directory / 'Cargo.lock'
            if lock.is_file():
                for item in tomllib.loads(lock.read_text())['package']:
                    if item.get('source', '').startswith('registry+'):
                        key = (item['name'], item['version'])
                        if key in packages and packages[key]['checksum'] != item['checksum']:
                            raise ValueError(f'inconsistent registry checksum: {key}')
                        packages[key] = item
        rust_lock = rust_source / 'Cargo.lock'
        for item in tomllib.loads(rust_lock.read_text())['package']:
            if item.get('source', '').startswith('registry+') and list((cargo_home / 'registry/cache').glob(f"*/{item['name']}-{item['version']}.crate")):
                packages[(item['name'], item['version'])] = item
        # Cargo resolves the entire rust-src workspace, including optional std
        # dependencies. Preserve the real sparse-index metadata as well as cached
        # archives; do not fabricate unused registry packages.
        for name in ('index', 'cache'):
            copy_tree(cargo_home / 'registry' / name, staging / 'cargo-registry' / name)
        for name in ('abi', 'tools/templates', 'tools/sdk/templates', 'docs/sdk'):
            copy_tree(repository / name, staging / name)
        for name in ('userspace/x86_64-unknown-none.json', 'tools/cargo-dunit', 'tools/dunit_cli.py', 'tools/dunit_manifest.py', 'tools/dunit_sdk.py', 'tools/sdk/bundle.py'):
            source = repository / name
            target = staging / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)
            target.chmod(0o755 if source.stat().st_mode & 0o111 else 0o644)
        copy_tree(sysroot, staging / 'sysroot')
        # These generated build-system files contain producer paths. Generate fresh
        # cross files at their destination with dunit_sdk.py; retain portable profile.
        cross = staging / 'sysroot/share/dunit-sdk/cross'
        for name in ('toolchain.cmake', 'meson.cross', 'config.site', 'sdk.json'):
            (cross / name).unlink(missing_ok=True)
        licenses = staging / 'licenses'
        licenses.mkdir()
        shutil.copyfile(repository / 'toolchains/dunit-musl/COPYRIGHT', licenses / 'musl-COPYRIGHT')
        (licenses / 'README.txt').write_text('Registry crates retain their original license files under vendor/.\nDunit project source has no declared project license; no license grant is inferred.\nThe static C sysroot includes musl; see musl-COPYRIGHT.\n')
        vendor = staging / 'vendor'
        vendor.mkdir()
        for package in sorted(packages.values(), key=lambda p: (p['name'], p['version'])):
            vendor_crate(package, cargo_home, vendor)
        metadata = {'schema_version': 1, 'target': 'x86_64-unknown-none', 'local_crates': local,
                    'rust_source_lock_sha256': digest(rust_lock),
                    'prerequisites': ['Python >= 3.11', 'rustup nightly with rust-src matching rust_source_lock_sha256', 'host C compiler/binutils for C applications'],
                    'native_execution': 'DUNIT_OS_CHECKOUT delegates to tools/qemu_test.py in a separate OS checkout',
                    'project_license': 'undeclared'}
        (staging / 'sdk-layout.json').write_text(json.dumps(metadata, indent=2, sort_keys=True) + '\n')
        (staging / 'README.txt').write_text('Dunit SDK: add tools/ to PATH, then cargo dunit new hello; cd hello; cargo dunit build.\nRust and its rust-src component are host prerequisites, not included.\nC: sysroot/bin/x86_64-dunit-cc app.c -ldunit_ui -o app\nrun/test require DUNIT_OS_CHECKOUT pointing to a compatible complete OS checkout.\nVerify/install: python3 tools/sdk/bundle.py verify SDK_DIR (or install ARCHIVE DESTINATION).\nSee licenses/README.txt before redistribution. Hashes establish integrity, not publisher authenticity.\n')
        for path in regular_files(staging):
            path.chmod(0o755 if path.stat().st_mode & 0o111 else 0o644)
        entries = {str(p.relative_to(staging)): {'sha256': digest(p), 'size': p.stat().st_size, 'mode': stat.S_IMODE(p.stat().st_mode)} for p in regular_files(staging)}
        (staging / MANIFEST).write_text(json.dumps({'schema_version': 1, 'files': entries}, sort_keys=True, indent=2) + '\n')
        (staging / MANIFEST).chmod(0o644)
        archive = Path(temporary) / 'sdk.tar.gz'
        with archive.open('wb') as sink, gzip.GzipFile(filename='', mode='wb', fileobj=sink, mtime=0) as compressed, tarfile.open(fileobj=compressed, mode='w') as tar:
            for path in regular_files(staging):
                info = tar.gettarinfo(str(path), arcname=str(path.relative_to(staging)))
                info.uid = info.gid = info.mtime = 0
                info.uname = info.gname = ''
                with path.open('rb') as source:
                    tar.addfile(info, source)
        # Hard-link publishes exclusively: never replace an existing destination.
        os.link(archive, output)
    return output


def verify(directory):
    data = json.loads((directory / MANIFEST).read_text())
    if data.get('schema_version') != 1 or not isinstance(data.get('files'), dict):
        raise ValueError('unsupported SDK manifest')
    actual = {str(p.relative_to(directory)): p for p in regular_files(directory) if p.relative_to(directory).as_posix() != MANIFEST}
    if set(actual) != set(data['files']):
        raise ValueError('SDK file set differs from manifest')
    for name, expected in data['files'].items():
        safe_name(name)
        path = actual[name]
        if expected != {'sha256': digest(path), 'size': path.stat().st_size, 'mode': stat.S_IMODE(path.stat().st_mode)}:
            raise ValueError(f'SDK file integrity mismatch: {name}')
    return len(actual)


def install(archive, destination):
    destination = destination.absolute()
    if destination.exists() or destination.is_symlink():
        raise ValueError(f'install destination already exists: {destination}')
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.sdk-install-', dir=destination.parent) as temporary:
        staging = Path(temporary) / 'sdk'
        staging.mkdir()
        total = 0
        seen = set()
        with tarfile.open(archive, 'r:gz') as tar:
            for member in tar:
                path = safe_name(member.name)
                if not member.isfile() or member.mode not in (0o644, 0o755) or member.name in seen:
                    raise ValueError(f'unsupported/duplicate archive member: {member.name}')
                total += member.size
                seen.add(member.name)
                if total > MAX_BYTES or len(seen) > MAX_FILES:
                    raise ValueError('SDK archive exceeds extraction limits')
                target = staging.joinpath(*path.parts)
                target.parent.mkdir(parents=True, exist_ok=True)
                with tar.extractfile(member) as source, target.open('xb') as sink:
                    shutil.copyfileobj(source, sink)
                target.chmod(member.mode)
        verify(staging)
        # mkdir reserves the destination without clobbering a racing install.
        destination.mkdir()
        try:
            os.rename(staging, destination)
        except BaseException:
            destination.rmdir()
            raise
    return destination


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    bundle = commands.add_parser('create')
    bundle.add_argument('output', type=Path)
    bundle.add_argument('--sysroot', type=Path, default=ROOT / 'build/dunit-sysroot')
    bundle.add_argument('--cargo-home', type=Path, default=Path(os.environ.get('CARGO_HOME', Path.home() / '.cargo')))
    bundle.add_argument('--rust-source', type=Path, required=True, help='installed nightly lib/rustlib/src/rust/library')
    check = commands.add_parser('verify')
    check.add_argument('directory', type=Path)
    extract = commands.add_parser('install')
    extract.add_argument('archive', type=Path)
    extract.add_argument('destination', type=Path)
    args = parser.parse_args()
    try:
        if args.command == 'create':
            print(create(args.output, args.sysroot, args.cargo_home, args.rust_source))
        elif args.command == 'verify':
            print(f'verified {verify(args.directory)} SDK files')
        else:
            print(install(args.archive, args.destination))
    except (OSError, ValueError, KeyError, tarfile.TarError) as error:
        parser.exit(1, f'[SDK] {error}\n')


if __name__ == '__main__':
    main()
