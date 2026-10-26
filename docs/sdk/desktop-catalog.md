# Installed desktop applications

DWM reads installed `dunit.toml` v1 files from
`/system/share/dunit/apps/<id>/dunit.toml` at startup and configuration reload.
The Rust `dunit_app_manifest` parser uses TOML with `core + alloc`; it does not
need a host OS backend. Known capability names are generated directly from
`abi/rights.abi`. Parsing and discovery never grant capabilities.

GUI entries supply their ID, display name, icon and `/app/<binary>` executable
to the existing userspace application registry. Launcher, dock, app groups,
shortcuts and startup then resolve those same IDs. Nothing is registered in
the kernel, and the compositor has no application-name dispatch table.

```toml
[meta]
schema_version = 2
[dock]
entries = ["example.monitor"]
[launcher]
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

CLI/TUI manifests are checked but are not inserted into the GUI registry: a
terminal launch adapter is not implemented yet. They continue to run through
`cargo dunit` and the terminal. GUI generation and SDK window helpers remain
separate M7 work.

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
