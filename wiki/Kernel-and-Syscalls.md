# Kernel & System Call ABI

This document defines the system call interface and application binary interface (ABI) between userspace applications and the **Dunit OS (Green Tea)** kernel.

---

## 1. Calling Convention

Dunit OS uses the native x86_64 `syscall` instruction. Legacy software interrupts (`int 0x80`) are not supported.

### Register Allocation

| Register | Role on Entry | Role on Return |
| --- | --- | --- |
| `RAX` | Syscall number | Return value (`>= 0` success, `< 0` `-errno`) |
| `RDI` | Argument 1 | Preserved or scratch |
| `RSI` | Argument 2 | Preserved or scratch |
| `RDX` | Argument 3 | Preserved or scratch |
| `R10` | Argument 4 | Preserved or scratch |
| `R8` | Argument 5 | Preserved or scratch |
| `R9` | Argument 6 | Preserved or scratch |
| `RCX` | Clobbered by CPU (saves userspace `RIP`) | Restored before return |
| `R11` | Clobbered by CPU (saves userspace `RFLAGS`) | Restored before return |

### Execution & Return Semantics
* Userspace invokes `syscall` with arguments loaded into registers.
* The CPU switches to the kernel stack specified in `TSS.RSP0` and transitions to Ring 0.
* Syscall arguments and CPU state are saved to the thread's stack.
* The kernel executes the handler and loads the return code into `RAX`.
* Control returns to userspace via `IRETQ` to maintain complete register safety.

---

## 2. Complete System Call Reference

The kernel exposes 41 system calls (indices 0 through 40) declared in `kernel/src/syscall/mod.rs`:

| № | Syscall Name | Arguments (`RDI`, `RSI`, `RDX`, `R10`, `R8`) | Return Value | Status |
|---|---|---|---|---|
| **0** | `exit` | `code: i32` | Does not return | ✅ Active |
| **1** | `fork` | *(none)* | `-ENOSYS` | ⛔ Reserved |
| **2** | `exec` | `path: *const u8, len: usize` | `-ENOSYS` | ⛔ Reserved (use `spawn_process`) |
| **3** | `read` | `fd: u32, buf: *mut u8, count: usize` | Bytes read / `-errno` | ✅ Active |
| **4** | `write` | `fd: u32, buf: *const u8, count: usize` | Bytes written / `-errno` | ✅ Active |
| **5** | `open` | `path: *const u8, len: usize, flags: u32` | File descriptor (`>= 3`) / `-errno` | ✅ Active |
| **6** | `close` | `fd: u32` | `0` / `-errno` | ✅ Active |
| **7** | `mmap` | `addr: usize, len: usize, prot: u32, flags: u32` | Allocated address / `-errno` | ✅ Active |
| **8** | `send_message` | `target_pid: u32, msg: *const u8, len: usize` | Bytes sent / `-errno` | ✅ Active |
| **9** | `receive_message`| `buf: *mut u8, len: usize` | Bytes received (`-EAGAIN` if empty) | ✅ Active |
| **10**| `get_framebuffer`| `info: *mut FbInfo` | `0` / `-errno` | ✅ Active |
| **11**| `draw_pixel` | `x: u32, y: u32, color: u32` | `0` | ✅ Active |
| **12**| `draw_rect` | `x: u32, y: u32, w: u32, h: u32, color: u32` | `0` | ✅ Active |
| **13**| `get_key` | *(none)* | Scancode / `-1` if queue empty | ✅ Active |
| **14**| `get_mouse_pos` | `x: *mut u32, y: *mut u32` | `0` / `-errno` | ✅ Active |
| **15**| `spawn_process` | `path: *const u8, len: usize` | Child `PID` / `-errno` | ✅ Active |
| **16**| `wait_process` | `pid: u32, status: *mut WaitStatus` | Child `PID` / `-errno` | ✅ Active |
| **17**| `get_pid` | *(none)* | Current `PID` | ✅ Active |
| **18**| `kill_process` | `pid: u32` | `0` / `-errno` | ✅ Active |
| **19**| `sleep` | `ms: u64` | `0` | ✅ Active |
| **20**| `debug_log` | `code: u64` | `0` | ✅ Active |
| **21**| `smoke_done` | `code: i32` | Reserved | ⚙️ Test harness |
| **22**| `get_cwd` | `buf: *mut u8, len: usize` | Path length / `-errno` | ✅ Active |
| **23**| `chdir` | `path: *const u8, len: usize` | `0` / `-errno` | ✅ Active |
| **24**| `yield` | *(none)* | `0` | ✅ Active |
| **25**| `get_terminal_cursor`| `info: *mut TerminalCursorInfo` | `0` / `-errno` | ✅ Active |
| **26**| `get_system_stats`| `info: *mut SystemStats` | `0` / `-errno` | ✅ Active |
| **27**| `readdir` | `path: *const u8, len: usize, ents: *mut DirEntry, max: usize` | Entry count / `-errno` | ✅ Active |
| **28**| `stat` | `path: *const u8, len: usize, stat: *mut FileStat` | `0` / `-errno` | ✅ Active |
| **29**| `thread_create` | `entry: usize, stack_top: *mut u8, arg: usize` | Child `TID` / `-errno` | ✅ Active |
| **30**| `thread_join` | `tid: u64, status: *mut WaitStatus` | Child `TID` / `-errno` | ✅ Active |
| **31**| `thread_exit` | `code: i32` | Does not return | ✅ Active |
| **32**| `get_tid` | *(none)* | Current `TID` | ✅ Active |
| **33**| `wait_event` | `timeout_ms: u64` (`0` = indefinite) | `0` / `-errno` | ✅ Active |
| **34**| `munmap` | `addr: usize, len: usize` | `0` / `-errno` | ✅ Active |
| **35**| `mprotect` | `addr: usize, len: usize, prot: u32` | `0` / `-errno` | ✅ Active |
| **36**| `shared_vm_create`| `len: usize` | Object ID / `-errno` | ✅ Active |
| **37**| `shared_vm_map` | `id: u32, addr: usize, prot: u32` | Mapped address / `-errno` | ✅ Active |
| **38**| `shared_vm_close` | `id: u32` | `0` / `-errno` | ✅ Active |
| **39**| `set_thread_pointer`| `fs_base: u64` | `0` / `-errno` | ✅ Active |
| **40**| `get_thread_pointer`| *(none)* | Hardware `FS_BASE` value | ✅ Active |

---

## 3. Memory Subsystem Syscalls

### `mmap` (Syscall 7)
Allocates page-aligned virtual memory regions:
```c
void *mmap(void *addr, size_t length, int prot, int flags);
```
* **Protection flags (`prot`)**:
  * `PROT_NONE` (`0x0`): Pages cannot be accessed.
  * `PROT_READ` (`0x1`): Pages may be read.
  * `PROT_WRITE` (`0x2`): Pages may be written.
  * `PROT_EXEC` (`0x4`): Pages may be executed. (Simultaneous `W+X` is strictly rejected for security).
* **Mapping flags (`flags`)**:
  * `MAP_PRIVATE` (`0x02`): Private copy-on-write mapping.
  * `MAP_ANONYMOUS` (`0x20`): Mapping is not backed by any file; pages are zero-initialized.
  * `MAP_GUARD` (`0x40`): Places non-present guard pages immediately preceding and following the allocated range. Guard pages trap stack and buffer overflows.

### `munmap` (Syscall 34)
```c
int munmap(void *addr, size_t length);
```
Releases virtual memory pages. If `addr` points to a guarded range allocated with `MAP_GUARD`, unmapping the entire usable region automatically frees both associated guard pages.

### `mprotect` (Syscall 35)
```c
int mprotect(void *addr, size_t length, int prot);
```
Modifies access permissions for an existing mapped page range.

---

## 4. Error Code Reference (`errno`)

Negative return values from system calls correspond to standard POSIX-compatible error numbers:

| Value | Name | Description |
|---|---|---|
| `-1` | `EPERM` | Operation not permitted |
| `-2` | `ENOENT` | No such file or directory |
| `-3` | `ESRCH` | No such process |
| `-4` | `EINTR` | Interrupted system call |
| `-5` | `EIO` | Input/output error |
| `-9` | `EBADF` | Bad file descriptor |
| `-10` | `ECHILD` | No child processes |
| `-11` | `EAGAIN` | Resource temporarily unavailable / try again |
| `-12` | `ENOMEM` | Cannot allocate memory |
| `-13` | `EACCES` | Permission denied |
| `-14` | `EFAULT` | Bad address pointer |
| `-16` | `EBUSY` | Device or resource busy |
| `-17` | `EEXIST` | File exists |
| `-19` | `ENODEV` | No such device |
| `-20` | `ENOTDIR`| Not a directory |
| `-21` | `EISDIR` | Is a directory |
| `-22` | `EINVAL` | Invalid argument |
| `-24` | `EMFILE` | Too many open files |
| `-28` | `ENOSPC` | No space left on device |
| `-38` | `ENOSYS` | Function not implemented |
| `-95` | `EOPNOTSUPP` | Operation not supported on socket/device |

