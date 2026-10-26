# Native app workflow

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

`new` creates Cargo.toml, the approved [dunit.toml v1](app-manifest-proposal.md),
and src/main.rs. `--kind tui`, `--name` and `--id` select metadata. It refuses
to overwrite existing directories. GUI metadata is valid, and existing GUI
binaries can be built, but GUI generation/run/test deliberately reports that
M7's window helper is not implemented yet.

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
unfinished. [Desktop discovery](desktop-catalog.md) now reads installed GUI
manifests; CLI/TUI desktop launching still needs a terminal adapter. Legacy
release app registration remains intact; SDK workflow packaging installs the
new manifest metadata.

```sh
cargo dunit new demo --kind gui --name "My window"
cargo dunit run --manifest-path demo/Cargo.toml
cargo dunit test --manifest-path demo/Cargo.toml
```

See the [window API and its initial limits](window.md). The starter is a themed
canvas; it does not implement the future Component/widget layer.

## Native tests

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

Verified on 26 October 2026: the full workflow passed with both TCG and KVM.
Positive runs booted, reported the expected output/test count and exited 0;
empty suites and returned errors exited 1, and panics exited 101, with the host
command failing as required. Calculator produced `42` for `7*6`, accepted
`exit`, and passed its two native cases. Live/installer initrd equality and the
installed manifest were checked. Host checks passed: 6 tooling, 8 image-profile,
3 macro and 10 runtime tests. The native ABI still has 89 syscalls.
The existing native SDK API/cooked-PTY Calculator, boot/runtime-stress/IPC and
monitor refresh/close marker budgets also passed. Warning count stayed 13/99.
