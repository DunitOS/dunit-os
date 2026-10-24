# Native M6 descriptor contracts

Approved in the task conversation. Numbers: Fcntl 86, Poll 87, Ioctl 88.
85 remains reserved for the historical DisplayOwner contract; it is not reused.
Generated syscall identifiers remain authoritative (`abi/syscalls.abi`).

## Fcntl(fd, command, argument)

| Command | Meaning |
|---|---|
| 0 | Get per-descriptor flags, bit 0 CLOEXEC |
| 1 | Set per-descriptor flags; reject bits other than bit 0 |
| 2 | Get shared status: READ=1, WRITE=2, APPEND=16, NONBLOCK=64 |
| 3 | Set APPEND/NONBLOCK; preserve access mode |
| 4 | Duplicate to lowest unused descriptor >= argument, clear CLOEXEC |
| 5 | Same, set CLOEXEC |
| 6 | Atomic pipe creation: fd carries NONBLOCK=64/CLOEXEC=128, argument points to two u32 descriptors |

Descriptor duplication shares open-description status/offset/terminal endpoint,
while CLOEXEC belongs to each descriptor. POSIX encodings are translated in
musl. Locks/ownership/signals are unsupported and return an explicit error.

## Poll(records, count, absolute_deadline_ns)

At most 1024 records, each 8 bytes: fd:i32, requested:u16, returned:u16,
little-endian. Requested READ=1/WRITE=2; returned additionally HUP=4, ERROR=8,
INVALID=16. A negative fd is ignored. Invalid descriptors produce INVALID;
other requested bits are rejected. Return value counts records with events.
Deadline 0 snapshots; u64::MAX waits indefinitely; otherwise CLOCK_MONOTONIC
nanoseconds. Readiness scan and wait-queue registration are atomic with respect
to interrupts; wake restarts the syscall with the original absolute deadline.
No per-frame userspace sleep loop replaces a kernel wait.

Regular files are immediately ready. Pipes and PTY input/output inspect actual
queue capacity and peer lifetime. Terminal stdio carries a shared endpoint in
its open description, so dup/inheritance preserve it independently of PID.
Master close gives EOF after buffered input and EPIPE for subsequent output;
slave EOF is observed after the last endpoint reference and buffered output.

This fd wait is not the undecided M7 heterogeneous IPC/timer/capability wait-set.
Termios subset: [native-terminal-abi.md](native-terminal-abi.md).

## Versioned SpawnArgs

Old shape is unchanged: path pointer/length, string block pointer/length, argc.
The old bounded length has its top bit clear. The sixth register is ignored
entirely in that case, including arbitrary nonzero values left by old wrappers.
An extended call sets length bit 63 and gives the string-section boundary in
argument six. The remaining block contains tagged action records.

Each header is six little-endian u32 words: tag, target fd, source fd, native
open flags, mode, path byte length; path bytes follow without NUL. Tags:
1 close, 2 dup2, 3 open, 4 chdir. Unused words must be zero. Limits: 64 actions,
16 argv + 16 env, 128 bytes per string, bounded copied block/path. Fchdir and
unsupported spawn attributes explicitly fail. Open mode is carried but POSIX
ownership/permission enforcement remains an unimplemented profile capability.

Actions run in order against a private child fd table/cwd. CLOEXEC cleanup
runs after actions, then ELF/argv/env/TLS are prepared while unscheduled. Only
a successful transaction publishes runnable state. Errors reap the prepared
child and its fd references. Filesystem effects of completed open actions are
not rolled back (matching spawn file-action side effects); parent descriptors
and cwd never change. Native privileged handles are not inherited through this
POSIX fd table. Unsupported attributes are rejected before child creation.

PTY argv uses the same explicit bit-63 opt-in in its existing block-length
argument; argument six is argv count only for opted-in calls. Old env-only
calls ignore that register. Search is userspace policy; native calls accept
literal paths rather than searching command names.

## File-backed private mapping

The existing Mmap fd/offset slots provide eager private snapshots, preserving
the shared file offset. Offset/alignment, lengths, read authority and W^X are
validated. The final partial file page is zero-filled; ranges beyond its page
are rejected. No shared mappings, demand paging, COW or SIGBUS are claimed.
