# Dunit OS

![Dunit OS](assets/images/banner.png)

Dunit OS is an experimental x86_64 operating system built from scratch. It uses
Green Tea Kernel (Rust `no_std`), a C/NASM hardware abstraction layer, and a
userspace desktop called DWM.

The kernel provides memory management, scheduling, drivers and process
isolation. The desktop, window manager and applications run as separate ring-3
ELF processes. Desktop policy stays in userspace.

## Quick start

Build on 64-bit Linux with `git`, `gcc`, `nasm`, `lld`, `xorriso`, `python3`,
`qemu-system-x86_64`, and nightly Rust with `rust-src`.

```bash
git clone --recurse-submodules https://github.com/DunitOS/dunit-os.git
cd dunit-os
rustup override set nightly
rustup component add rust-src
./build_iso.sh
```

The bootable ISO is written to `build/microkernel.iso`.

Common build targets:

- `make iso`: terminal ISO.
- `make iso-dwm`: graphical desktop ISO.
- `make disk-image`: BIOS/UEFI raw disk image.

Build and test the desktop through the QEMU harness:

```bash
python3 tools/qemu_test.py --build --config limine_test_gui.conf \
  --markers-file tools/gui_smoke_markers.json \
  --ready-marker 'gui_server: desktop ready' --timeout 300 --linger 8
```

## DWM desktop

DWM runs in the `gui_server` userspace software compositor. Its desktop can be
configured without recompiling the OS:

- Multiple independent panels on any screen edge, with configurable size,
  alignment, spacing, auto-hide and window reservations.
- Nested components: launcher, workspaces, application lists, tray, titles,
  clocks, system monitors, buttons and indicators.
- Pinned and running applications, plus application folders with grid, list or
  fan presentation.
- TrueType text with configurable font paths and component font sizes.
- Transparency, blur, shadows, rounded corners and DSS style transitions.
- Configurable keybindings and per-application window rules for workspace,
  geometry, floating state, opacity and animation.
- Window dragging, resizing, minimize/maximize/restore and a window switcher.
- Digital and analog clocks, periodic widget updates and Lua-driven indicators.

Desktop configuration has three layers:

- TOML defines components, panels, applications, keybindings and window rules.
- DSS controls appearance, dimensions, spacing, states and transitions.
- Lua provides layouts, event handlers, desktop commands and animation policies.

Settings includes panels, typography, widgets, animations, Lua scripts,
keybindings, window rules, themes and desktop profiles. Structural settings use
validated source editors. Theme overlays and complete desktop imports are
separate operations.

Configuration reloads without restarting the desktop. Candidates are validated
and prepared before applying; a failed reload retains the last working scene.
Schema-1 configurations remain supported.

The system document is `/system/share/dwm/default.toml`. A complete user profile
at `/persist/config/dwm.toml` takes precedence. Persistence across reboot requires
a persistent `/persist` mount.

## Lua scripting

DWM includes Lua 5.5.1 and the versioned Lua SDK 2. Layout scripts control window
placement. Event scripts can react to window creation, closing, focus,
minimizing, restoring and maximizing, as well as workspace changes, panel
interaction, launcher actions, widget updates and configuration changes.

For example, fade in newly created windows:

```lua
local dwm = require("dwm")
assert(dwm.api_version == 2)

dwm.on("window::created", function(win)
    dwm.animate(win, {
        property = "opacity",
        from = 0,
        to = 1,
        duration = 250,
        easing = "ease_out"
    })
end)
```

Animations include fade, slide, scale, spring and Genie. Scripts can register
custom easing curves and animate window, component, panel, launcher, workspace
and notification properties. Lua submits descriptors; Rust interpolates frames
using a monotonic clock. Lua does not run on every compositor frame.

Scripts run in separate processes with limits on memory, execution, message
size and event queues. The API exposes checked desktop commands and widget
updates, without arbitrary access to kernel memory or compositor capabilities.
Script failures retain native fallback behavior.

The bundled macOS-style event example can be selected in the active document's
`[scripts]` section:

```toml
[scripts]
enabled = true
events = "/assets/dwm/scripts/events/macos.lua"
```

Complete profiles are available in Settings. From the Dunit terminal, activate
the Frost example with:

```text
exec /app/setup import-user /assets/dwm/frost-desktop.toml
```

Examples and documentation:

- [Frost desktop](assets/dwm/frost-desktop.toml): one full-width bottom panel.
- [Multi-panel desktop](assets/dwm/multi-panel-desktop.toml): an additional
  auto-hidden utility panel with an analog clock.
- [DSS example](assets/dwm/styles/frost.dss).
- [Lua event example](assets/dwm/scripts/events/macos.lua).
- [Configuration reference](docs/architecture/dwm-shell.md).
- [Lua SDK reference](docs/DWM_LUA_SDK.md).
- [Implementation and verification report](docs/DWM_ENGINE_V2_REPORT.md).

## Kernel and runtime

- Limine bootloader with BIOS and UEFI support.
- Single-core preemptive scheduling with FPU/SSE state preservation.
- Isolated processes, kernel threads and static `pthread` support.
- Per-process page tables, guard pages, `mmap`, `munmap`, `mprotect` and W^X.
- Native Dunit syscall ABI and a dedicated static musl libc port.
- AHCI SATA and VirtIO block drivers with GPT support.
- MemFS root populated from an external initrd; DunitFS v2 on `/persist`.
- `install.dunit` installation with an EFI System Partition and persistent data.

## Applications and tools

- GUI Terminal, File Manager, Calculator, Settings and System Stats.
- `dsh`: command history, tab completion and quoted arguments/path execution.
  Search paths come from `/assets/exec-path`.
- `dufetch`: memory, uptime, display and process information.
- `calc`, `bmp_viewer` and `fsck_dunit`.
- Tiny C Compiler (`tcc`) for compiling C programs inside Dunit.

Compile a static C application on the host:

```bash
build/dunit-sysroot/bin/dunit-cc -O2 -o build/userspace/hello hello.c
```

Compile and run inside Dunit:

```text
exec /system/bin/tcc /assets/tcc/hello.c -o /tmp/hello
exec /tmp/hello
```

## Screenshots and demos

Limine bootloader:

![Limine bootloader](assets/images/bootloader.gif)

Terminal mode:

![Terminal mode](assets/images/terminalmode.gif)

DWM desktop:

![DWM desktop](assets/images/dwm.gif)

## Current limitations

Dunit is under active development. Execution is currently single-core. Networking,
audio, dynamic linking, signals and a secure entropy service are not yet ready.
The C runtime profile is static; unsupported semantics return explicit errors.

DWM uses software rendering. Large blur regions and many animated surfaces cost
CPU and memory. Scene limits are 64 panels, 512 components and 16 nesting levels.
Genie animates an owned window snapshot. Clocks currently use uptime rather
than RTC calendar time.

The milestone order is M6 (musl/toolchain), M7 (SDK), M8 (network), M9
(applications), M10 (browser). See the [roadmap](DUNIT_OS_TECHNICAL_ROADMAP.md)
and [M6 results](docs/m6-readiness.md) for acceptance status and open decisions.

## License

MIT.
