# Application manifest v1

Approved by the project owner on 26 October 2026 (M7.2 and roadmap §31).
This document specifies the accepted format; implementation progress is tracked
in the roadmap.

`dunit.toml` sits beside Cargo.toml:

```toml
schema_version = 1

[application]
id = "calc"
name = "Calculator"
kind = "tui"
binary = "calc"
# icon = "/assets/icons/apps/calc.svg"
capabilities = []
```

- `id`: stable lowercase ASCII identifier (letters, digits, `_`, `-`, `.`),
  beginning with a letter, at most 64 bytes. No slashes or traversal components.
- `name`: nonempty UTF-8 display name, without control characters.
- `kind`: `cli`, `tui` or `gui`. CLI/TUI do not require an icon. GUI does.
- `binary`: exact Cargo binary target; optional only when Cargo metadata has
  exactly one binary. IDs and executable names need not match.
- `icon`: optional absolute VFS path. It names an installed asset; a host path
  is never copied into an image implicitly.
- `capabilities`: unique names from the current native rights manifest. These
  declare requirements; they do not grant handles, privilege or filesystem
  access. Existing native runtime checks remain authoritative. Unknown names
  fail validation. No new permission mechanism is part of this format.

Unknown keys, missing required fields and unsupported schema versions are
errors with a path/field diagnostic. Build/run must fail before modifying an
image. There is no silent acceptance of a newer format.

The accepted source is copied into the image at
`/system/share/dunit/apps/<id>/dunit.toml`; the ELF remains under `/app/<binary>`.
The desktop registration contract uses its name/icon/exec metadata in userspace;
launcher/dock discovery is not implemented in the first tooling slice. Existing
user desktop layout, pinning and explicit presentation overrides retain their
meaning; no application name or path is added to kernel code. Legacy bundled
applications keep their current registry until they gain manifests.

`cargo dunit` uses Cargo metadata for targets and the existing initrd packer,
image builder and `tools/qemu_test.py` for packaging/boot/testing. It must not
introduce another QEMU launcher or initrd format. Its first CLI/TUI tooling
slice must explicitly report GUI generation/runtime gaps until the SDK window
helper exists; it must not generate a pretend working GUI application.
