# SDK & Application Development

Dunit OS provides a complete native development ecosystem supporting both **Rust** (`cargo-dunit` toolchain) and **C/C++** (`dunit-cc` and musl libc). This guide explains how to create, build, test, and package applications for Dunit OS.

---

## 1. Rust Application Development

The native Rust SDK allows developers to build high-performance, memory-safe graphical, console, and terminal applications running directly on the Dunit microkernel.

### The `cargo-dunit` Subcommand

The primary developer tool is `cargo-dunit` (`tools/dunit_cli.py`). It manages project templates, compilation against the custom target specification, and automated execution in QEMU.

```bash
# Add cargo-dunit to your PATH or symlink to ~/.cargo/bin/
export PATH="/path/to/dunit-os/tools:$PATH"
```

#### CLI Commands

| Command | Usage | Description |
|---|---|---|
| `new` | `cargo dunit new <path> --kind <cli\|tui\|gui>` | Generates a new project skeleton with boilerplate and manifests. |
| `build` | `cargo dunit build [--manifest-path Cargo.toml]` | Compiles the application targeting `x86_64-unknown-none`. |
| `run` | `cargo dunit run [--accel auto\|kvm\|tcg] -- [args]` | Compiles the binary, generates a test image, and boots it in QEMU. |
| `test` | `cargo dunit test` | Executes native in-guest integration tests. |

---

### Application Manifest (`dunit.toml`)

Every application defines a `dunit.toml` manifest specifying its identity, desktop integration, and execution capabilities:

```toml
[package]
id = "org.dunit.calculator"
name = "Calculator"
version = "1.0.0"
authors = ["Dunit Developers"]
description = "Lightweight desktop calculator"
category = "Accessories"

[app]
binary = "calc"
icon = "/assets/icons/apps/calc.png"

[window]
width = 360
height = 480
resizable = false
title = "Calculator"

[rights]
framebuffer = true
ipc = true
input = true
```

---

### Rust Application Skeleton

Native applications run in a `#![no_std]` environment with access to dynamic allocations through `alloc`:

```rust
#![no_std]
#![no_main]

extern crate alloc;

use core::time::Duration;
use dunit::window::{Window, WindowEvent, IN_KEY, IN_QUIT};

#[dunit::main]
fn main() -> dunit::Result<()> {
    dunit::println!("[App] Starting Hello World GUI application...");

    // Create a 640x480 desktop window managed by DWM / gui_server
    let mut window = Window::open(640, 480)?;

    let mut running = true;
    while running {
        // Poll input and window events non-blockingly
        while let Ok(Some(event)) = window.next_event(Duration::from_millis(0)) {
            match event {
                WindowEvent::CloseRequested => {
                    running = false;
                }
                WindowEvent::Input(input) => {
                    if input.kind == IN_QUIT {
                        running = false;
                    } else if input.kind == IN_KEY {
                        dunit::println!("[App] Key pressed: {}", input.x);
                    }
                }
                _ => {}
            }
        }

        // Render application graphics into the window backbuffer
        let fb = window.framebuffer_mut();
        // Clear background with dark slate color (0x1E1E2E)
        fb.fill(0x001E1E2E);

        // Present damage region to the compositor
        window.present()?;

        // Yield CPU time slice to other processes
        dunit::process::sleep(16);
    }

    dunit::println!("[App] Exiting cleanly.");
    Ok(())
}
```

---

## 2. C / C++ Application Development

Dunit OS supports standard C/C++ development using a dedicated musl libc sysroot and the `dunit-cc` compiler driver.

### The `dunit-cc` Compiler Wrapper

`dunit-cc` is located at `/path/to/dunit-os/build/dunit-sysroot/bin/dunit-cc`. It wraps GCC or Clang with the target sysroot:
* Uses `-nostdinc` and `-isystem` pointing to Dunit's musl C headers.
* Generates static, position-dependent binaries (`-static -no-pie -nostdlib -nostartfiles`).
* Links `crt1.o`, `crti.o`, `libc.a`, `libgcc.a`, and `crtn.o`.

```bash
# Compiling a standalone C application:
dunit-cc -O2 -Wall -c main.c -o main.o
dunit-cc main.o -o my_c_app
```

---

### Native C UI Component Toolkit (`<dunit/ui.h>`)

C programs can construct rich native user interfaces that render via the system `gui_server` without writing manual pixel rasterizers.

To use the toolkit, link with `-ldunit_ui` and include `<dunit.h>`:

```c
#include <dunit.h>
#include <dunit/ui.h>

typedef struct {
    int counter;
} AppState;

// Callback to construct UI node tree
static int app_build_ui(void *user, DunitUiNodeV1 *nodes, size_t capacity) {
    AppState *state = (AppState *)user;

    // Node 0: Root Column container
    nodes[0] = (DunitUiNodeV1){
        .kind = DUNIT_UI_NODE_COLUMN,
        .parent = DUNIT_UI_ROOT,
    };

    // Node 1: Label
    char label_text[64];
    snprintf(label_text, sizeof(label_text), "Clicks: %d", state->counter);
    nodes[1] = (DunitUiNodeV1){
        .kind = DUNIT_UI_NODE_TEXT,
        .parent = 0,
        .text = label_text,
        .text_len = strlen(label_text),
    };

    // Node 2: Button
    nodes[2] = (DunitUiNodeV1){
        .kind = DUNIT_UI_NODE_BUTTON,
        .parent = 0,
        .text = "Click Me",
        .text_len = 8,
        .message = 1001, // Custom action ID
    };

    return 3; // Total nodes
}

// Callback to handle user interactions
static int app_handle_action(void *user, uint64_t message) {
    AppState *state = (AppState *)user;
    if (message == 1001) {
        state->counter++;
        return 0; // Return 0 to trigger UI rebuild
    }
    return 0;
}

int main(int argc, char **argv) {
    AppState state = { .counter = 0 };
    DunitUiCallbacksV1 callbacks = {
        .abi_version = DUNIT_UI_ABI_VERSION,
        .struct_size = sizeof(DunitUiCallbacksV1),
        .build = app_build_ui,
        .update = app_handle_action,
    };

    return dunit_ui_run_v1(argc, argv, NULL, 320, 240, &callbacks);
}
```

---

## 3. Application Packaging & System Integration

To integrate an application into the Dunit OS desktop environment:

1. **Copy Binary**: Place the compiled ELF executable into `/app/<name>`.
2. **Deploy Manifest**: Save `<name>.toml` in `/system/share/manifests/`.
3. **Register in DWM**: Add the application identifier to the launcher entries in `/system/share/dwm/default.toml`:
   ```toml
   [launcher]
   entries = ["sec_sys", "gui_terminal", "sec_games", "doom", "sec_soft", "my_app"]
   ```
4. **Deploy Assets**: Store application assets and icons in `/system/share/<name>/` and `/assets/icons/apps/`.

