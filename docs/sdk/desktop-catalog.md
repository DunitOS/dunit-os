# Installed desktop applications

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

CLI/TUI manifests now enter the same registry with their original kind. Startup,
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

```sh
cargo test --manifest-path runtime/dunit-app-manifest/Cargo.toml
DUNIT_ROOT="$PWD" cargo test --manifest-path userspace/dwm_settings/Cargo.toml
python3 tools/test_dunit_cli.py
python3 tools/test_sdk_catalog.py --accel tcg
python3 tools/test_sdk_gui_fixture.py console --accel tcg
```

The native fixture has ID `example.monitor` and executes the existing
`gui_stat` binary. It checks dock/launcher/startup resolution without desktop
application tables, edits and rereads the name, rejects an invalid schema,
restores it, and then starts, maximizes, restores and closes a real monitor
window through the shared SDK window helper. Scalar theme/font consumers now
use the same catalog-aware configuration resolver as the compositor. The probe
binary and installed test manifest belong to the full developer
workflow; the sample desktop document ships with the other DWM examples. Release
application lists stay explicit. Shared TOML fixtures exercise both Python
packaging and Rust parsing.

MemFS now returns ENOENT for an absent directory and ENOTDIR for a regular file
in both directory readers. The optional-catalog native regression checks both
cases and verifies that the legacy desktop remains valid without any manifest
folder (`tools/sdk_catalog_absent_markers.json`).

Verified on 26 October 2026: catalog resolution, metadata reread, invalid-schema
rejection/recovery and real monitor startup/refresh/close passed on KVM and TCG.
The native absent-catalog/errno check, SDK API and cooked-PTY Calculator, full
DWM/Lua/Genie/settings regression, and multi-client GUI resize/restore budget
passed. Host tests passed: 28 DWM, 2 manifest parser, 8 packaging/tooling and
8 image-profile cases. ABI validation still reports 89 syscalls.

Repeated acceptance runs exposed a second kernel bug on both KVM and TCG:
a kernel interrupt could observe a still-pending user-fault request after the
user escape frame had already returned. CPU capture showed CPL0 halted in
`syscall_escape_user_fault.halt` with the escape frame inactive. The interrupt
handler now permits scheduling/fault escapes only for a CPL3 interrupted frame.
The `boot-smoke-tests` regression injects a real CPL0 INT3 in that exact window,
without changing the pending fault or faking timer ticks; execution must return
and print `[USER-FAULT-IRQ] OK`. The production fix is unconditional; the probe
is compiled only in the existing smoke-test profile. Its separate marker budget
is `tools/user_fault_irq_markers.json`. No automatic retry or relaxed marker
was added.

After the fault-return fix, 20 complete thread-test processes passed on KVM and
20 on TCG, each including the forced kernel-interrupt probe. The entire
cargo-dunit workflow then passed on TCG (including empty/error/panic rejection
and Calculator), followed by the installed catalog, DWM engine, SDK API and
GUI resize/restore regressions. Compiler warnings stayed at 13/99.

Verified console increment on 2 November 2026: the native `console` fixture
passed on TCG. It launches a CLI with a space-containing executable path through
startup, a TUI through launcher keyboard navigation, and a legacy shell through
a configured desktop binding. CLI/TUI assert exact argv and `hello` from real
keyboard input, write stdout/stderr through their PTYs, and exercise normal child
exit and cancellation on close. The legacy shell executes `echo shellok`. All
three terminal windows exit 0; required boot markers pass and GUI faults remain
forbidden. Screenshots capture final CLI output, ANSI TUI output and the shell.
The unchanged installed-GUI catalog regression also passed. Configuration host
tests passed 40 checks, terminal model tests 10; ABI generation remains in sync
at 90 syscalls, 25 errno and 7 rights.
