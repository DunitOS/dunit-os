# Porting Applications to Dunit OS

Porting existing C/C++ and Rust software to **Dunit OS (Green Tea)** is straightforward thanks to the POSIX-aligned userspace ABI, musl libc integration, and native windowing primitives. This guide provides technical instructions, solutions to common architectural pitfalls, and real-world case studies based on successful ports.

---

## 1. Porting Architecture & Toolchain

### Target Specification
* **Architecture**: `x86_64` (AMD64 / Intel 64).
* **Binary Format**: Static ELF 64-bit executable (`-static -no-pie -nostdlib -nostartfiles`).
* **C Toolchain**: `dunit-cc` (compiler driver wrapping target headers and musl libc).
* **Rust Toolchain**: `cargo-dunit` targeting `x86_64-unknown-none`.

```bash
# Typical C compilation command
dunit-cc -O2 -Wall -c app.c -o app.o
dunit-cc app.o -o /path/to/dwm_bundle/bin/app
```

---

## 2. Musl Libc Environment & POSIX Support

Dunit OS includes a custom port of **musl libc**. Most standard C library functions work out of the box:
* **Standard I/O (`stdio.h`)**: `fopen`, `fread`, `fwrite`, `fseek`, `ftell`, `fclose`, `printf`, `snprintf`.
* **Memory Management (`stdlib.h`)**: `malloc`, `calloc`, `realloc`, `free`, `qsort`, `abs`.
* **String Utilities (`string.h`)**: `strlen`, `strcpy`, `strncpy`, `strcmp`, `strncmp`, `memcpy`, `memset`, `memmove`.
* **Math Library (`math.h`)**: `sin`, `cos`, `tan`, `sqrt`, `floor`, `ceil`, `fabs`, `pow`.

### Unsupported POSIX Calls & Workarounds

| Unsupported POSIX Feature | Dunit OS Behavior | Recommended Workaround |
|---|---|---|
| `fork()` / `exec()` | Returns `-ENOSYS` | Design single-binary applications or invoke child processes using the `spawn_process()` system call. |
| POSIX Signals (`signal`, `sigaction`) | Minimal stub support | Implement cooperative event loops; poll input events using `window.next_event()` or `get_key()`. |
| Dynamic Linking (`dlopen`, `.so`) | Not supported | Link all dependencies and libraries statically (`.a`). |
| Network Sockets (`socket`, `connect`) | Under development | Rely on local file I/O or shared memory IPC (`shared_vm_create`). |

---

## 3. Critical Porting Pitfalls & Solutions

During the porting of Doom, Quake, Minecraft C Edition, and Tracker, several recurring low-level issues were identified and resolved:

### Pitfall 1: 64-bit Pointer Expansion & Memory Pools (The Quake Bug)
* **The Problem**: Classic 32-bit DOS/Win95 codebases assumed 4-byte pointers (`sizeof(void*) == 4`). In 64-bit long mode on Dunit OS, all pointers and memory alignments expand to 8 bytes.
* **The Symptom**: Static memory pools (such as Quake's Hunk memory `parms.memsize = 8 * 1024 * 1024`) overflow during map loading (`start.bsp`, `e1m3.bsp`), leading to heap corruption and page faults (`CR2 = 0x1000b7c198`).
* **The Fix**: Expand initial memory pools to at least 24 MiB (`24 * 1024 * 1024`) in `quakegeneric.c`.
* **Bit-shift Overflow**: Watch for 32-bit signed shift overflows. In Quake's `r_edge.c:678`, the expression `2000 << 24` overflows a 32-bit signed integer into negative `0xD0000000`, breaking edge sorting. Replace with the safe maximum `0x7FFFFFFF`.

---

### Pitfall 2: Mouse Button Release (`IN_UP`) Handling (The Minecraft MCED Bug)
* **The Problem**: In `gui_server/src/main.rs`, mouse release events (`IN_UP`) historically forwarded a fixed button index of `0` (left button) regardless of which button was released. Releasing the right mouse button (button `1`) never dispatched code `1`.
* **The Symptom**: The game's `mouse_right` flag remained `true` indefinitely, causing the player to place blocks uncontrollably whenever cooldown expired.
* **The Fix in Application Code**:
  1. In the input handler, whenever receiving any mouse-up event (`!down`), reset both button states:
     ```c
     if (!down) {
         app->mouse_left = false;
         app->mouse_right = false;
     }
     ```
  2. Immediately after executing an action (such as placing a block), reset `app->mouse_right = false` to prevent repeat triggering without a fresh click.

---

### Pitfall 3: Audio Tracker Format Verification (The Tracker Bug)
* **The Problem**: Compact module players like `pocketmod` strictly support 4-channel Amiga ProTracker `.MOD` files (verifying the `M.K.` or `4CHN` signature at byte offset 1080). FastTracker II `.XM` files fail header validation, returning `0` and causing `NO MODULE LOADED` warnings.
* **The Fix**: Verify audio module formats using `file song.mod` before embedding. Use authentic 4-channel ProTracker files (e.g. *Space Debris* by Markus Kaarlonen).
* **In-Memory Loading**: Instead of relying on relative filesystem paths (`fopen("song.mod")`), embed sound files directly into the executable using `include_bytes!` and pass memory pointers directly:
  ```rust
  static EMBEDDED_SONG: &[u8] = include_bytes!("../../song.mod");
  tracker_core_init_mem(EMBEDDED_SONG.as_ptr(), EMBEDDED_SONG.len() as i32);
  ```

---

### Pitfall 4: Relative Paths in Desktop Autostart
* **The Problem**: When launched from the DWM desktop dock or autostart menu, a process's current working directory may default to `/` rather than `/app/`. Calls like `fopen("assets/font.dat")` fail.
* **The Fix**: Always resolve assets using absolute paths (e.g. `/app/share/<app>/font.dat`) or embed static resources directly into the binary.

---

## 4. Case Studies of Ported Software

| Application | Origin | Porting Details & Architecture |
|---|---|---|
| **Doom** | `doomgeneric` | Uses software renderer blitting to a 320x200 32bpp framebuffer surface. Maps keyboard scancodes to Doom keys. Loads original `doom1.wad`. |
| **Quake** | `quakegeneric` | 64-bit software rasterizer. Hunk memory expanded to 24 MiB. Fixed signed shift in `r_edge.c`. Supports shareware `pak0.pak`. |
| **Minecraft C Edition** | `mced` | Pure C voxel rendering engine. Uses software raycasting and polygon rasterization. Added mouse button release safety overrides. |
| **Game Boy** | `gnuboy` | Full Z80/LR35902 CPU emulator, PPU tile renderer, and APU sound emulation. Supports `.gb` and `.gbc` ROMs. |
| **GrafX2** | `grafx2` | Classic pixel-art drawing program. Features 256-color palette manipulation, brush tools, and image editing. |
| **Lite XL** | `lite-xl` | Lightweight, highly responsive text and code editor powered by an embedded Lua scripting engine. |
| **Video Player** | Custom | Reads raw frame streams and blits video frames synchronized with audio clocks. |
| **Tracker** | `pocketmod` | 4-channel Amiga ProTracker music player with real-time oscilloscopes, pattern matrix visualization, and sample displays. |

