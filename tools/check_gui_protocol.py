#!/usr/bin/env python3
"""Reject copied live control-channel constants outside their canonical codec.

Golden wire tests may contain literal bytes. Production Rust must import the
codec instead. Derive the values from control.rs so this gate owns no ABI values.
"""
import ast
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
AUTHORITY = Path('protocols/gui-v1/src/control.rs')
DECLARATION = re.compile(
    r'pub\s+const\s+(\w+_MAGIC)\s*:\s*u32\s*=\s*'
    r'u32::from_le_bytes\s*\(\s*\*b"([^"\\]{4})"\s*\)\s*;')
TOKEN = re.compile(
    r'(?P<line>//[^\n]*)|(?P<block>/\*)|'
    r'(?P<raw>(?P<byte>b?)r(?P<hash>\#*)"(?P<body>.*?)"(?P=hash))|'
    r'(?P<string>b?"(?:\\.|[^"\\])*")|'
    r'(?P<number>\b(?:0[xX][0-9a-fA-F_]+|0[bB][01_]+|0[oO][0-7_]+|[0-9][0-9_]*)'
    r'(?:[ui](?:8|16|32|64|128|size))?\b)|'
    r'(?P<identifier>\b[A-Za-z_]\w*\b)', re.DOTALL)
COMMENT_EDGE = re.compile(r'/\*|\*/')
SUFFIX = re.compile(r'[ui](?:8|16|32|64|128|size)$')


def tokens(source):
    """Keep literal/identifier tokens; skip comments, including nested blocks."""
    cursor = 0
    while match := TOKEN.search(source, cursor):
        cursor = match.end()
        if match.group('line') is not None:
            continue
        if match.group('block') is not None:
            depth = 1
            while depth:
                edge = COMMENT_EDGE.search(source, cursor)
                if edge is None:
                    raise ValueError('Unterminated Rust block comment')
                depth += 1 if edge.group() == '/*' else -1
                cursor = edge.end()
            continue
        yield match


def violations(source, names, values):
    previous = None
    for token in tokens(source):
        value = None
        if token.group('raw') is not None and token.group('byte'):
            value = token.group('body').encode('utf-8')
        elif token.group('string') is not None and token.group('string').startswith('b"'):
            try:
                value = ast.literal_eval(token.group('string'))
            except (SyntaxError, ValueError):
                pass  # Rust validates spellings outside Python's byte grammar.
        elif token.group('number') is not None:
            spelling = SUFFIX.sub('', token.group('number')).replace('_', '')
            radix = {'0x': 16, '0b': 2, '0o': 8}.get(spelling[:2].lower(), 10)
            value = int(spelling, radix)
        name = token.group('identifier')
        if (previous == 'const' and name in names) or value in values:
            line = source.count('\n', 0, token.start()) + 1
            yield line, token.group()
        previous = name


def check(root=ROOT):
    authority = root / AUTHORITY
    source = authority.read_text()
    public = {token.start() for token in tokens(source) if token.group('identifier') == 'pub'}
    definitions = [match.groups() for match in DECLARATION.finditer(source)
                   if match.start() in public]
    declared = [match.group(1) for match in re.finditer(r'pub\s+const\s+(\w+_MAGIC)\b', source)
                if match.start() in public]
    if not definitions or len(definitions) != len(declared):
        raise ValueError('Canonical control magic declarations must use explicit four-byte tags')
    names = {name for name, _ in definitions}
    tags = {tag.encode('ascii') for _, tag in definitions}
    if len(names) != len(definitions) or len(tags) != len(definitions):
        raise ValueError('Duplicate canonical control magic declarations')
    values = tags | {int.from_bytes(tag, 'little') for tag in tags}
    sources = set()
    for parent in ('runtime', 'userspace', 'protocols'):
        for folder in (root / parent).glob('*/src'):
            sources.update(folder.rglob('*.rs'))
    # system_apps has an additional package level.
    for folder in (root / 'userspace/system_apps').glob('*/src'):
        sources.update(folder.rglob('*.rs'))
    sources.update((root / 'kernel/src').rglob('*.rs'))
    failures = []
    for path in sorted(sources):
        if path == authority:
            continue
        for line, token in violations(path.read_text(), names, values):
            failures.append(f'{path.relative_to(root)}:{line}: copied control constant {token}')
    if failures:
        raise ValueError('\n'.join(failures))
    return len(definitions), len(sources)


if __name__ == '__main__':
    try:
        count, sources = check()
        print(f'GUI control ABI OK: {count} canonical tags, {sources} production Rust files, 0 copies')
    except ValueError as error:
        raise SystemExit(str(error))
