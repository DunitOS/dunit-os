"""Accepted dunit.toml v1 validation; capabilities remain declarative."""
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
import re
import tomllib
import unicodedata


@dataclass(frozen=True)
class Application:
    source: Path
    id: str
    name: str
    kind: str
    binary: str
    icon: str | None
    capabilities: tuple[str, ...]


def load_manifest(path: Path, binaries: list[str], rights: Path) -> Application:
    return parse_manifest(path.read_text(encoding="utf-8"), path, binaries, rights)


def parse_manifest(content: str, path: Path, binaries: list[str], rights: Path) -> Application:
    def fail(field, reason):
        raise ValueError(f"{path}: {field}: {reason}")

    def keys(table, allowed, field):
        if not isinstance(table, dict):
            fail(field, "expected a table")
        unknown = table.keys() - allowed
        if unknown:
            fail(field, f"unknown keys: {', '.join(sorted(unknown))}")

    if len(content.encode("utf-8")) > 64 * 1024:
        fail("root", "manifest exceeds 64 KiB")
    try:
        data = tomllib.loads(content)
    except tomllib.TOMLDecodeError as error:
        fail("TOML", str(error))
    keys(data, {"schema_version", "application"}, "root")
    if type(data.get("schema_version")) is not int or data['schema_version'] != 1:
        fail("schema_version", "expected version 1")
    app = data.get("application")
    keys(app, {"id", "name", "kind", "binary", "icon", "capabilities"}, "application")
    identifier = app.get("id")
    if not isinstance(identifier, str) or not re.fullmatch(r'[a-z][a-z0-9_.-]{0,63}', identifier):
        fail("application.id", "expected a lowercase ASCII id, 1..64 bytes, without traversal")
    name = app.get("name")
    if not isinstance(name, str) or not name.strip() or any(unicodedata.category(c) == 'Cc' for c in name):
        fail("application.name", "expected a nonempty display name without control characters")
    kind = app.get("kind")
    if kind not in ("cli", "tui", "gui"):
        fail("application.kind", "expected cli, tui or gui")
    binary = app.get("binary")
    if binary is None and len(binaries) == 1:
        binary = binaries[0]
    if not isinstance(binary, str) or binary not in binaries or not re.fullmatch(r'[A-Za-z0-9_.-]+', binary) or binary in ('.', '..'):
        fail("application.binary", "expected an exact, unambiguous Cargo binary target")
    icon = app.get("icon")
    if icon is not None:
        if (not isinstance(icon, str) or not icon.startswith('/')
                or icon.startswith('//') or icon == '/'
                or str(PurePosixPath(icon)) != icon
                or '..' in PurePosixPath(icon).parts
                or any(unicodedata.category(c) == 'Cc' for c in icon)):
            fail("application.icon", "expected a normalized absolute VFS path")
    if kind == "gui" and icon is None:
        fail("application.icon", "required for gui applications")
    capabilities = app.get("capabilities")
    known = {line.split()[1] for line in rights.read_text().splitlines() if line.strip() and not line.lstrip().startswith('#')}
    if not isinstance(capabilities, list) or any(not isinstance(c, str) for c in capabilities):
        fail("application.capabilities", "expected an array of rights names")
    if len(set(capabilities)) != len(capabilities) or set(capabilities) - known:
        fail("application.capabilities", "expected unique names from abi/rights.abi")
    return Application(path, identifier, name, kind, binary, icon, tuple(capabilities))


def installed_manifest(app: Application) -> str:
    """Materialize Cargo's resolved binary; installed readers cannot query Cargo."""
    import json
    def quote(value):
        return json.dumps(value, ensure_ascii=False)
    result = f'schema_version = 1\n\n[application]\nid = {quote(app.id)}\n'
    result += f'name = {quote(app.name)}\nkind = {quote(app.kind)}\nbinary = {quote(app.binary)}\n'
    if app.icon is not None:
        result += f'icon = {quote(app.icon)}\n'
    result += 'capabilities = [' + ', '.join(quote(cap) for cap in app.capabilities) + ']\n'
    if len(result.encode('utf-8')) > 64 * 1024:
        raise ValueError(f'{app.source}: installed manifest exceeds 64 KiB')
    return result
