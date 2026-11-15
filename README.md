# Dunit OS

Dunit is an experimental x86_64 operating system with a Rust kernel, C/NASM HAL,
Limine BIOS/UEFI boot, a recovery terminal and a userspace desktop. Applications
use its native syscall ABI. Static musl and TinyCC support C programs; the native
Rust SDK uses no_std + alloc.

## Desktop and applications

DWM runs with the compositor in gui_server. TOML configures panels, applications,
shortcuts and window rules; DSS controls appearance; separate Lua 5.5.1 workers
provide layouts, events, widget commands and motion. Invalid reloads preserve
the previous desktop. Settings exposes themes/profiles, panels and configuration.
Bundled apps include Terminal, Files, Calculator, Settings and System Monitor.
Text clipboard and scoped file transfers support selection, drag/drop, copy/move
and opening files through compatible dock entries.

The [desktop manual](docs/architecture/desktop.md) describes configuration,
examples, Lua API, limits and verification. A user profile at
`/persist/config/dwm.toml` overrides `/system/share/dwm/default.toml`.
[Frost](assets/dwm/frost-desktop.toml) and the
[multi-panel example](assets/dwm/multi-panel-desktop.toml) are bundled profiles:

```text
exec /app/setup import-user /assets/dwm/frost-desktop.toml
```

## Build and run

Use 64-bit Linux with git, gcc, nasm, lld, xorriso, Python 3.11+, QEMU and
nightly Rust with rust-src. Image checks need mtools; UEFI tests need OVMF.

```bash
git clone --recurse-submodules https://github.com/DunitOS/dunit-os.git
cd dunit-os
rustup override set nightly
rustup component add rust-src
./build_iso.sh
```

`build/dunit_dwm.iso` contains the desktop, Lua and C toolchain.
`build/dunit_minimal.iso` contains the terminal and base utilities. Both use the
same kernel/native ABI and retain an installer payload. See the
[image guide](docs/build-images.md) for composition and release checks.
All automated builds/boots use the shared harness:

```bash
python3 tools/qemu_test.py --build --config limine_test_gui.conf \
  --markers-file tools/gui_smoke_markers.json \
  --ready-marker 'gui_server: desktop ready' --timeout 300 --linger 25
```

## Current limits

Application scheduling remains UP. AP initialization and ownership probes do
not establish concurrent application dispatch. Root is MemFS loaded from external
initrd; DunitFS provides persistent data at `/persist`, not a disk-backed system
root. Networking, audio, full USB, dynamic linking, signals and secure entropy
remain incomplete. Desktop rendering is software; large effects are expensive.
Panels/components are bounded, Genie animates a frozen snapshot, and desktop
clocks show uptime.

[Current state](docs/architecture/current-state.md) records implementation,
review findings and verification limits. The
[technical roadmap](DUNIT_OS_TECHNICAL_ROADMAP.md) tracks features; the
[performance roadmap](DUNIT_OS_PERFORMANCE_ROADMAP.md) tracks OP0–OP10 gates.

## Develop applications

The [SDK guide](docs/sdk/README.md) covers native Rust/C applications, components,
input, layout, themes and transfers. `cargo dunit` creates/builds/runs/tests
CLI/TUI/GUI applications and validates installed manifests. The
[native ABI](abi/README.md) defines lower-level contracts, and the
[Lua manual](docs/architecture/lua-runtime.md) covers scripts and on-device builds.

Read the [contributor guide](.github/CONTRIBUTING.md) before changing code.
Include behavior, reproduction commands, results and remaining limits in changes.

## License

MIT. Bundled upstream code/assets retain their own licenses and provenance.
