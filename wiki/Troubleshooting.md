# Troubleshooting & Diagnostics

This guide provides troubleshooting solutions for common boot failures, kernel panics, CPU exceptions, application crashes, and SDK compilation errors in **Dunit OS (Green Tea)**.

---

## 1. Diagnostics & Log Collection

The primary diagnostic tool in Dunit OS is the **serial console**. All subsystems, drivers, and user applications write debug diagnostics directly to the 16550 UART serial port (`COM1` / `0x3F8`).

### Enabling Serial Output in QEMU
Always run QEMU with serial output directed to stdout or a file:
```bash
# Terminal stdout:
qemu-system-x86_64 -cdrom build/dunit.iso -serial stdio

# File logging:
qemu-system-x86_64 -cdrom build/dunit.iso -serial file:qemu_debug.log
```

### Healthy Boot Sequence Checklist
Verify that your serial log contains the standard operational markers in chronological order:
1. `[BOOT] START` & `[KERNEL] START` (Limine handoff)
2. `[KERNEL] HHDM setup` & `[KERNEL] framebuffer ready`
3. `[HAL] OK` & `GDT loaded` & `IDT loaded with 256 entries`
4. `[PMM] OK` & `[VMM] OK` & `[HEAP] OK (PMM-backed, growable)`
5. `[SCHED] UP round-robin init: timer-preemption=on`
6. `[INITRD] Initrd: 1126 file(s) loaded`
7. `[VFS] Root MemFS mounted at /`
8. `[DRV] PS/2: Keyboard detected on port 1` & `Mouse detected on port 2`
9. `[DAMAGE] partial frame OK` (DWM compositor actively running)

---

## 2. Kernel Exceptions & Panics

### Page Fault (`#PF`, Vector 14)

A Page Fault occurs when the CPU attempts to access an unmapped or protected virtual address.

* **Check `CR2` (Faulting Address)**:
  * If `CR2` is close to zero (e.g. `0x0000000000000008`): Null pointer dereference in application or driver code.
  * If `CR2` is just below userspace stack: Stack overflow. Increase the stack size in `dunit.toml` or your application assembly stub.
  * If `CR2` is `0x0000001000b7c198`: Memory pool boundary overflow (e.g. Quake 64-bit hunk exhaustion).
* **Decode the Error Code (P/W/U bits)**:
  * **Bit 0 (`P`)**: `0` = Page not present; `1` = Protection violation (e.g. writing to read-only page).
  * **Bit 1 (`W/R`)**: `0` = Access was a Read; `1` = Access was a Write.
  * **Bit 2 (`U/S`)**: `0` = Fault occurred in Kernel (Ring 0); `1` = Fault occurred in Userspace (Ring 3).

### General Protection Fault (`#GP`, Vector 13)

* **Non-canonical Address Access**: x86_64 addresses must have bits 48 through 63 matching bit 47. Attempting to access an address like `0x0000ABCD00000000` triggers `#GP`.
* **MSR Violation**: Writing an invalid value to a Model-Specific Register (such as `EFER` or `LSTAR`).
* **Privilege Violation**: Attempting to execute privileged instructions (`cli`, `hlt`, `in`, `out`) from Ring 3 without proper IOPL rights.

---

## 3. Application Runtime Issues

### Tracker: Displays "NO MODULE LOADED"
* **Cause**: `pocketmod` strictly supports 4-channel Amiga ProTracker `.MOD` files (checking for `M.K.` / `4CHN` signature at offset 1080). FastTracker II `.XM` files fail format verification.
* **Fix**: Use genuine 4-channel `.MOD` files (e.g. *Space Debris* by Markus Kaarlonen). Embed via `include_bytes!` to prevent working directory lookup failures.

### Minecraft (MCED): Stuck Right Mouse Button / Infinite Block Placement
* **Cause**: In `gui_server:1738`, mouse release (`IN_UP`) events historically passed a hardcoded button code of `0` (left button). Right mouse button release events (code `1`) were never received by apps.
* **Fix**: In application input routines, reset both mouse buttons on any mouse-up event:
  ```c
  if (!down) {
      app->mouse_left = false;
      app->mouse_right = false;
  }
  ```

### Quake: Page Fault on Loading Maps (`start.bsp`)
* **Cause**: 64-bit pointers take 8 bytes instead of 4, causing classic 8 MiB hunk allocations (`parms.memsize = 8 * 1024 * 1024`) to overflow. Furthermore, `2000 << 24` in `r_edge.c:678` causes a signed 32-bit integer overflow into negative `0xD0000000`.
* **Fix**: Expand hunk memory to 24 MiB (`24 * 1024 * 1024`) and replace the edge sentinel limit with `0x7FFFFFFF`.

### Application Exits with "File Not Found" on Startup
* **Cause**: When launched from the DWM desktop dock, the process's current working directory is `/` rather than `/app/`. Calls like `fopen("game.dat", "rb")` fail.
* **Fix**: Specify absolute paths (`fopen("/app/game.dat", "rb")`) or bundle assets directly into the binary.

---

## 4. Compilation & Toolchain Issues

### `dunit-cc: host include directory is outside the Dunit sysroot`
* **Cause**: Compiler flags included host directories (`-I/usr/include`).
* **Fix**: Remove `-I/usr/include` and rely strictly on `-isystem $SYSROOT/include`.

### Linker Error: `relocation R_X86_64_32S against ... can not be used`
* **Cause**: Object files compiled without position-independent code were linked into a PIE binary.
* **Fix**: Ensure `-static -no-pie` is passed to the linker.

### Rust Error: `can't find crate for 'core'`
* **Cause**: The Rust nightly toolchain is missing the target source code.
* **Fix**:
  ```bash
  rustup component add rust-src --toolchain nightly
  ```

