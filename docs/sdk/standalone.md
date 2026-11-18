# Standalone SDK

The distribution contains the native Rust CLI/TUI/GUI source SDK, its transitive
local crates, actual registry crate sources, target specification, ABI manifests,
application templates and the prepared static C sysroot. It includes the generated
C headers and `libdunit_ui.a`. No Rust toolchain, kernel, OS image, QEMU or competing
VM runner is bundled. The Rust SDK remains `no_std + alloc`.

Python 3.11+, rustup nightly and rust-src are host prerequisites. The producer's
rust-src `library/Cargo.lock` hash is recorded in `sdk-layout.json`; use the same
nightly sources for reproducible offline `build-std=core,alloc` builds. The SDK
vendors application lockfile dependencies and cached dependencies from that
rust-src lockfile. It also ships the real Cargo registry index/cache snapshot: the
Rust build-std resolver needs metadata for optional std workspace dependencies,
even when compiling only core and alloc. It does not claim arbitrary nightly versions are compatible.
C builds require a host C compiler and binutils; the relocated wrapper selects
its own sysroot. The existing static C profile and its unsupported operations
remain unchanged.

## Produce and install

Prepare the normal C sysroot and generated C SDK through the repository workflow
before packaging. Packaging never rebuilds it or accesses the network. The
producer must already have the registry archives named by the SDK lockfiles in
Cargo's cache; missing archives or checksum mismatches are errors.

```sh
python3 tools/sdk/bundle.py create /tmp/dunit-sdk.tar.gz \
  --sysroot build/dunit-sysroot --cargo-home "$HOME/.cargo" \
  --rust-source "$HOME/.rustup/toolchains/nightly-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library"
python3 tools/sdk/bundle.py install /tmp/dunit-sdk.tar.gz /opt/dunit-sdk
python3 /opt/dunit-sdk/tools/sdk/bundle.py verify /opt/dunit-sdk
```

Use a writable install destination. Both archive output and installation refuse
an existing destination. Identical input contents and executable bits produce
identical tar.gz bytes: entries are sorted and timestamps, owners and gzip header
metadata are normalized. Installation validates paths, rejects duplicate names,
links, devices, special permissions and oversized archives, and verifies all
file hashes, sizes, modes and the exact file set before publishing the directory.
A failed extraction leaves the destination absent and removes its temporary tree.
The manifest detects corruption; it does not authenticate a publisher. Obtain the
archive and its expected digest through a trusted distribution channel.

The bundle retains registry packages' actual license files and musl's COPYRIGHT.
Dunit project source currently has no declared project license. The bundle records
that fact and does not infer permission to redistribute it or invent a license.

## External applications

```sh
export PATH="/opt/dunit-sdk/tools:$HOME/.cargo/bin:$PATH"
cargo dunit new /tmp/hello
cd /tmp/hello
cargo dunit build
```

`new --kind tui` and `new --kind gui` use the same native SDK and templates as
checkout mode. Generated Cargo manifests use an explicit relative path to the
installed SDK's `runtime/dunit` crate. Its transitive SDK dependencies already
use relative paths. The generated workflow needs no global Cargo patch, including
during build-std resolution. Install the SDK at any prefix before creating an app;
projects never reference the producer checkout.

`cargo dunit` forces offline resolution. Registry metadata and archives are copied
into the external project's `target/.dunit-cargo-home`; extraction and Cargo writes
occur there, preserving the installed SDK's integrity. An existing external
CARGO_HOME is not needed. Direct `cargo build` does not supply the SDK's target or
`build-std` flags; use `cargo dunit build`.

Cargo resolves relative dependency paths from the project's manifest directory.
Moving the SDK and project together while preserving their relative locations
keeps those paths valid. Moving either independently requires updating the
project's `dunit.path` and PATH as appropriate. Existing project manifests are never
rewritten automatically. Manually version-declared SDK dependencies remain
supported through invocation-time patches for their requested SDK entry crates,
including package renames. Cargo may report those patches as unused while
resolving its separate build-std workspace; the generated path-based workflow
avoids that unrelated patch scope.

For C applications:

```sh
/opt/dunit-sdk/sysroot/bin/x86_64-dunit-cc app.c -ldunit_ui -o app
python3 /opt/dunit-sdk/tools/dunit_sdk.py \
  --sysroot /opt/dunit-sdk/sysroot --output /tmp/dunit-cross
```

CMake/Meson/config.site files contain the selected installed sysroot path and must
be regenerated after a move. Producer-specific cross files are excluded from the
bundle; the portable static profile remains available.

## Native execution

```sh
export DUNIT_OS_CHECKOUT=/path/to/compatible/dunit-os
cargo dunit run
cargo dunit test
```

Standalone run/test requires this explicit setting and checks the checkout has
the existing harness, kernel manifest and Makefile and matching public ABI
manifests before building. It reports the delegated harness path. Application
compilation uses the standalone SDK; staging, image construction, kernel build,
locking and guest execution use the configured checkout's existing workflow and
`tools/qemu_test.py`. That checkout must be compatible with the bundled runtime;
ABI manifest equality alone cannot prove full implementation compatibility.
Run/test cannot boot independently of a complete OS checkout. Checkout mode
continues to use its original local dependencies and native harness.

Host regression checks:

```sh
python3 tools/test_dunit_cli.py
python3 tools/test_sdk_bundle.py
```

Release acceptance additionally builds external CLI/TUI/GUI projects with an empty
external Cargo home after relocating the installed SDK, checks all resolved local
package paths belong to the installed SDK or the application, and exercises a
native fixture through the configured checkout. Compilation does not establish
runtime correctness.

## Recorded acceptance

The [follow-up receipts](../verification/multitrack-progress-2026-11-17.json)
record offline relocated CLI/TUI/GUI builds, native CLI and static C filesystem/UI
checks, and GUI run plus 200 real frame commits through the delegated harness.
The original 32 MiB PMM growth budget remains unchanged. Initial desktop resources
are prepared before client bootstrap, and initial slot presentation precedes
FRAME_DONE. Aggregate diagnostics reduced measured 200-frame PMM growth from
34,062,336 to 856,064 bytes on the TCG runner; this fixes startup ordering and is
not a hardware rendering-speed benchmark.
