# Application tooling

The SDK is tied to a Dunit source checkout and uses its existing image builder,
initrd packer and `tools/qemu_test.py`.

## Application manifests

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

Accepted metadata is installed into the image at
`/system/share/dunit/apps/<id>/dunit.toml`; the ELF remains under `/app/<binary>`.
Packaging materializes Cargo's resolved `binary` in that installed manifest;
source comments and ordering are not preserved.
The desktop registration contract uses its name/icon/exec metadata in userspace;
[Desktop discovery](tooling.md#desktop-catalog) uses this installed metadata. Existing
user desktop layout, pinning and explicit presentation overrides retain their
meaning; no application name or path is added to kernel code. Legacy bundled
applications keep their current registry until they gain manifests.

`cargo dunit` uses Cargo metadata for targets and the existing initrd packer,
image builder and `tools/qemu_test.py` for packaging/boot/testing. It must not
introduce another QEMU launcher or initrd format. GUI generation uses the native themed component window; CLI/TUI generation
uses the same native entry and stream APIs.

## Cargo workflow

`cargo dunit` builds and exercises native Rust CLI/TUI/GUI applications on the
Dunit kernel. The SDK uses `no_std + alloc`; no application code needs an ABI
entry point, `unsafe`, a panic handler or hand-written QEMU markers.

Use the repository's `tools` directory in PATH (or symlink `tools/cargo-dunit`
into a directory already in PATH). The command requires Python 3.11+, rustup
nightly with rust-src, and the normal Dunit image/QEMU dependencies. This first
SDK is tied to its source checkout: generated Cargo dependencies point to that
checkout. A standalone distributable SDK is future work.

```sh
export PATH="/path/to/dunit-os/tools:$HOME/.cargo/bin:$PATH"
cargo dunit new hello
cd hello
cargo dunit build
cargo dunit run
cargo dunit test
```

`new` creates Cargo.toml, the approved [dunit.toml v1](tooling.md#application-manifests),
and src/main.rs. `--kind tui`, `--name` and `--id` select metadata. It refuses
to overwrite existing directories. GUI generation/run/test uses the native
window helper and installs metadata for compositor launching.

`build` checks the manifest, resolves the exact Cargo binary target through
Cargo metadata, then cross-compiles using the same target and build-std options
as the OS build. Unsupported schemas, unknown fields, ambiguous binaries and
unknown rights fail before an image is touched. `capabilities` declares
requirements; it never grants privileges.

`run` and `test` use Cargo's executable artifact path, stage that ELF at
`/app/<binary>` and normalized installed metadata at
`/system/share/dunit/apps/<id>/dunit.toml`, then call `tools/qemu_test.py --build`.
The existing packer applies explicit file overlays; Live and installer payloads
use the same initrd. The archive format and ABI remain unchanged. SDK commands
serialize their image builds because the checkout has one shared build folder;
do not run an independent OS build concurrently.

Execution is headless and expects the application to finish successfully.
`--accel tcg` selects software emulation; `--timeout 240` controls guest budgets.
All commands accept `--manifest-path /path/to/Cargo.toml`. `run` can drive a TUI:

```sh
cargo dunit run --manifest-path /path/to/dunit-os/userspace/system_apps/calc/Cargo.toml \
  --input-ready 'calc> ' --input '7*6' --input exit
cargo dunit test --manifest-path /path/to/dunit-os/userspace/system_apps/calc/Cargo.toml
```

`--input` types complete printable ASCII lines through the existing harness.
`--input-ready` waits for a prompt; otherwise input starts after a short delay.
Guest arguments follow `--`; the recovery console currently supports only
nonempty ASCII words without whitespace, and the SDK rejects other arguments.
GUI `run` is a headless smoke: it starts the real compositor with the selected
installed app, waits for the presented window, sends the configured close
shortcut after two seconds and requires that child's actual exit code 0.
GUI `test` autostarts the native test binary in the same desktop; tests may open
one window for the process. The generated case publishes 201 frames.
GUI launch currently rejects guest arguments and terminal input.
Interactive QEMU windows, arbitrary quoted arguments and packaging assets are
unfinished. [Desktop discovery](tooling.md#desktop-catalog) reads GUI/CLI/TUI manifests; console
applications run through the configured terminal adapter. Legacy
release app registration remains intact; SDK workflow packaging installs the
new manifest metadata.

```sh
cargo dunit new demo --kind gui --name "My window"
cargo dunit run --manifest-path demo/Cargo.toml
cargo dunit test --manifest-path demo/Cargo.toml
```

GUI runs and native GUI fixtures use the existing `dwm` image composition with
`limine_sdk_terminal.conf`: the desktop and SDK overlay fit the fixed installer
ESP without packaging every unrelated CLI/libc test executable. Kernel/ABI
smoke runs retain the full composition. Both paths still use `tools/qemu_test.py`.

See the [window API and its limits](layout-window.md#native-windows). The starter uses a typed root
component and the [shared SDK theme](themes.md#themes). Its native buffer-lifetime test
still issues 201 real low-level commits; it does not substitute unchanged-view
updates, which correctly skip presentation.

### Native tests

Templates include Rust's custom test framework attributes and `#[dunit::main]`.
Add `#[dunit::test]` to a safe no-argument function returning `()` or
`dunit::Result<()>`. The compiler discovers cases, including cases in modules;
there is no source scanner or manually maintained list. The SDK runs them in
one guest process, printing case names and a final count.

Each run requires the guest boot marker and the selected program's exit code 0.
Tests additionally require the SDK success count and forbid failure/panic
markers. An assertion panic exits 101; a returned error or empty test suite
exits 1. Those runs make `cargo dunit test` fail. The current runner stops at the
first failing case. Tests share process state; per-case isolation, filters,
ignored tests and parallel execution are not implemented. Use the generated
`#[dunit::test]` attribute, rather than Rust's host `#[test]`.

```sh
python3 tools/test_dunit_cli.py
python3 tools/test_image_profiles.py
python3 tools/test_dunit_workflow.py --accel tcg
python3 tools/test_dunit_gui_workflow.py --accel tcg
```

The CLI/TUI workflow covers new/build/run/test, a native file roundtrip and installed
manifest read, empty/error/panic rejection, and both native tests and real TUI
execution of Calculator. Logs are saved under build/sdk-workflow/; all guest
launches still go through qemu_test.py.

## Desktop catalog

DWM reads installed `dunit.toml` v1 files from
`/system/share/dunit/apps/<id>/dunit.toml` at startup and configuration reload.
The Rust `dunit_app_manifest` parser uses TOML with `core + alloc`; it does not
need a host OS backend. Known capability names are generated directly from
`abi/rights.abi`. Parsing and discovery never grant capabilities.

Installed entries supply their ID, display name, icon and `/app/<binary>` executable
to the existing userspace application registry. Launcher, dock, app groups,
shortcuts and startup then resolve those same IDs. Nothing is registered in
the kernel, and the compositor has no application-name dispatch table.

```toml
[meta]
schema_version = 2
[dock]
entries = ["example.monitor"]
[launcher]
terminal = "/app/gui_terminal"
entries = ["example.monitor"]
[startup]
entries = []
```

This desktop document does not repeat the manifest's name, icon or executable.
Optional `[application.example.monitor]` fields still explicitly override the
manifest defaults, preserving existing user configuration semantics. Empty
lists remain empty; explicit lists retain their order and act as selections,
so discovery does not silently pin a new application. When lists are omitted,
normal registry defaults apply. A profile without application tables inherits
the packaged legacy registry as before, alongside installed GUI manifests.
Legacy applications can keep their current desktop tables until migrated.

CLI/TUI manifests enter the same registry with their original kind. Startup,
dock and launcher share the same launch path: GUI runs directly; CLI/TUI runs
the configured `[launcher] terminal` executable with distinct argv entries
`[adapter, "--execute", executable]`. No command string or shell interpolation is
used. The adapter path is desktop policy, not an application-name table in the
compositor. The packaged default selects `/app/gui_terminal`; explicit profiles
may select another adapter implementing this command-line interface.

The adapter field is optional for GUI-only selections. Selecting a CLI/TUI entry
without it rejects the complete configuration; console entries are never launched
invisibly. The path must be normalized and absolute. Applications not selected in
dock/launcher/startup do not require an adapter. Existing explicit list ordering,
empty selections and legacy GUI tables retain their meaning. CLI/TUI entries
without an icon use the existing derived-label fallback. Settings-only saves
preserve the independent launcher table.

`gui_terminal --execute executable [arguments...]` runs that command on its PTY;
running it without arguments still opens `dsh` with the configured prompt. Each
argument is forwarded separately, including paths with spaces. A completed child
is reaped once and its final output remains visible until the window closes.
Closing a terminal with a running child terminates and reaps that owned child.
The existing SpawnArgs syscall creates the adapter unprivileged; PTY spawn and
manifest discovery grant no additional rights. No ABI, manifest-schema or gui-v1
wire change is involved.

Discovery checks normalized IDs and paths, supported schemas, unknown keys,
required metadata, duplicate/unknown rights and existence of executable files.
The directory ID must match the manifest ID. Files are bounded to 64 KiB; a
catalog may contain at most 256 manifests, and the resolved desktop still has
the existing 32-application limit. Oversized catalogs fail instead of being
silently truncated. A missing catalog is valid and preserves the legacy path.
A malformed catalog rejects the complete candidate; live DWM keeps its previous
working configuration. The loader reports the manifest path and parse reason.
The existing icon fallback still applies if an icon cannot be rendered.

Source manifests may omit `binary` when Cargo has one binary target. Packaging
materializes the resolved target in installed metadata, since the guest cannot
query Cargo. Source comments and field ordering are not part of installed
metadata. Source and installed manifests use the same v1 schema.

`validate_document` remains a pure legacy-document resolver.
`validate_document_with_manifests` is its pure installed-manifest counterpart;
`validate_installed_document` reads the installed catalog and is used by Settings,
setup and full-document saves/imports. Syntax and known scalar validation run
before those operations read the catalog.
