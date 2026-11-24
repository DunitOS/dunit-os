# Native Dunit ABI

The kernel uses native Dunit syscall numbers and records. POSIX types are
translated by musl; public kernel structs are never replaced with Linux structs.
The ABI query currently reports major version 0. ThreadExit v1 names an opt-in
argument encoding, not a global ABI-major change.

## Authoritative definitions

`syscalls.abi`, `errno.abi` and `rights.abi` define numbers, positive errno
magnitudes and capability bits. `transfers.abi` defines file/transfer operations.
Generated C headers in `include/dunit/` and Rust modules in `userspace/libdunit`
are committed outputs; do not edit them by hand. Kernel enums/constants must
agree with their manifests. The generator verifies that agreement:

```bash
python3 tools/gen_abi.py
python3 tools/gen_abi.py --check
python3 tools/gen_transfer_values.py --check
```

## Calling convention

| Register | Meaning |
|---|---|
| rax | Number on entry, signed result on return |
| rdi, rsi, rdx, r10, r8, r9 | Arguments 0 through 5 |
| rcx, r11 | Clobbered by syscall; not arguments |

Nonnegative results indicate success; negative results are negated POSIX errno
magnitudes. Entry is `syscall`; return uses IRETQ. HAL-private dispatch return
sentinels are not userspace results. User flags are preserved while DF is cleared
for kernel execution; see [entry flags](../docs/architecture/kernel-entry-flags.md).

`AbiQuery` selectors: 0 gives ABI major version; 1 gives highest number plus one;
2 queries whether a specified syscall is enabled in this build. Reserved numbers
and smoke-only calls are not proof of supported production behavior.
Handle rights attenuate on transfer and cannot widen.

## Process entry and ELF

```text
rsp -> argc: u64
       argv pointers, NULL
       envp pointers, NULL
       auxv {type: u64, value: u64}, ending {AT_NULL, 0}
       padding, NUL-terminated strings and AT_RANDOM bytes
```

RSP is 8 mod 16 for a no-libc function-style `_start`; rdi=argc, rsi=argv,
rdx=envp. Musl crt1 realigns its stack. Auxv includes AT_PAGESZ=4096,
AT_SECURE=0, AT_RANDOM and AT_PHDR/PHENT/PHNUM/ENTRY for static TLS. A bounded
exact program-header copy is supplied on the initial stack when no load-backed
table exists; invalid PT_PHDR declarations are rejected. Skip unknown auxv
entries. Bootstrap random bytes are not secure entropy.

ELF64/LE/x86_64 ET_EXEC static executables are supported. PT_LOAD permissions
obey W^X; PT_TLS initializes x86_64 Variant II static TLS independently per TID.
PT_INTERP, ET_DYN/PIE, dynamic loader/DTV and PT_GNU_RELRO are unsupported.
Run `tools/check_userspace_elf.py` before stripping: it validates bounds, entry,
static segments, non-executable stack and named undefined symbols. Stripped
files cannot prove symbol information that has been removed; OSABI metadata
alone does not prove native syscall usage.

## Memory and records

VM page size is 4096. Anonymous private mappings are zero-filled. Protection
supports NONE, R, RW and RX; W+X is rejected. MAP_GUARD (bit 6) adds an unmapped
page on each side. Full usable-range unmap releases guards; partial guarded
unmap is rejected. A fixed guarded address names the reservation start, with
usable return address one page later. Shared VM uses rights-bearing handles
and independent mapping lifetimes: unmap and handle close are separate releases.

Public records are repr(C), little-endian x86_64. DirEntry carries a 64-byte name,
name length and type; FileStat carries type/size. File types are 1 file, 2 directory,
3 device. WaitStatus is two i32 words: kind/code. Kinds: 0 exit, 1 page fault,
2 general protection, 3 invalid opcode, 4 divide by zero, 5 unknown fault,
-1 empty and -2 prepared child. Generated headers and typed bindings define
complete current layouts and bounds; no copied historical syscall table is kept.
A path is bounded to 256 bytes, a native user-copy to 64 KiB, readdir to 64 entries
per call and IPC messages to 256 bytes. Safe wrappers use smaller transfer chunks.

## Threads

Additional threads share process address space/cwd/fds and own GPR, kernel stack,
FXSAVE and static TLS. ThreadCreate validates executable entry and writable,
16-byte-aligned user stack top; the return slot is at stack_top-8. Entry must
terminate with ThreadExit. A thread fault is isolated from the remaining process.
Caller-owned user stack stays live until the thread completes and is joined.

ThreadExit=31 decodes arg0 high32 exactly 1 as opt-in auto-reap; low32 is the
i32 exit code. Legacy signed i32 encodings (high32 0 or 0xffffffff) retain joinable
status. Main-thread opt-in returns EINVAL. Heavy stack/TLS owners retire after
HAL escapes the exiting stack; ThreadJoin=30 consumes joinable status. Auto-reap
also discards status and must not have a joiner. This encoding was approved
24 October 2026 and adds no syscall number.

## Descriptors and spawn

Reserved descriptors are 0 stdin, 1 stdout, 2 stderr; ordinary fd allocation
starts at 3 with a limit of 1024. Open flags: READ=1, WRITE=2, CREATE=4, TRUNC=8,
APPEND=16, EXCL=32, NONBLOCK=64, CLOEXEC=128. Access/unsupported bits are checked.

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

The mixed native wait-set is described below; the terminal subset follows.

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

## Terminal ioctl

`Ioctl(fd, operation, record, record_len)` uses Dunit operation numbers and bounded
little-endian records, not Linux requests/termios structures. Operations:

| op | record | Meaning |
|---|---|---|
| 0 | null, length 0 | Probe an attached PTY stdio descriptor |
| 1 | 8 bytes | Get rows, columns, pixel width, pixel height (four u16) |
| 2 | 8 bytes | Set those dimensions on the attached endpoint |
| 3 | 4 bytes | Get pending input byte count (u32) |

Unset dimensions are zero, meaning unknown; the kernel does not invent terminal
geometry. State is shared by the PTY endpoint's stdin/stdout/stderr and inherited/duplicated descriptors, independent of process identity. The process must
possess a valid descriptor referring to that terminal. Unknown descriptors return
EBADF, ordinary files/pipes return ENOTTY, bad record lengths return EINVAL,
invalid user memory returns EFAULT, unsupported terminal operations return
EOPNOTSUPP. Records are copied and validated before mutation.

musl translates TIOCGWINSZ/TIOCSWINSZ/FIONREAD and uses the public ioctl adapter
for isatty. Canonical line editing, terminal echo, baud rates, process groups,
TCGETS/TCSETS, drain/flush and signal generation are **not implemented**; requests
are rejected explicitly. Returning a fabricated termios record would claim a
line discipline the PTY does not have. Geometry change signals are likewise not
claimed. GUI master geometry publication awaits a descriptor mechanism for the
master endpoint; this subset does not overload an unrelated file descriptor
with a PTY table identifier.

## Native wait-set

WaitSetWait=89 observes own-inbox IPC, fd readiness, monotonic timers and child
termination in a bounded blocking call. It is non-consuming and level-triggered,
with call-scoped registration. `abi/wait-set` defines records and
`libdunit::wait_set::wait` supplies the safe binding. See
[SDK foundation](../docs/sdk/native-api.md#wait-set) for limits and semantics.

## Queued pointer input (M7, approved 31 October 2026)

`ReadPointerEvent` (90) reads one explicitly selected little-endian event using an owned
`INPUT_MASTER` handle. EAGAIN means no queued event; invalid output does not
consume an event. `abi/pointer-input` shares the codec between kernel and
`libdunit::pointer::read`. Button transitions survive render stalls; pure motion
may coalesce. Overflow and ownership handover explicitly reset the consumer's
press/capture baseline. `GetMouseState` (60), including its wheel accumulator,
remains compatible. See the [complete contract](../docs/sdk/input.md#queued-pointer-input).

The compatible modifier extension approved 2 November 2026 keeps length 32 as
exact v1. Length 40 returns those same bytes plus a u32 Shift/Ctrl/Alt/Super mask
at 32..36 (bits 0..3), captured at enqueue, and zero reserved bytes at 36..40.
Length 56 adds v3: the v2 prefix, signed i32 device dx/dy at offsets 40/44,
a u32 relative-source flag at 48, and four zero reserved bytes at 52. Absolute
sources have zero deltas. `read_with_motion` requests v3 and preserves movement
beyond display bounds. See [pointer modes](../docs/sdk/input.md#pointer-modes).
Other lengths return EINVAL. `libdunit::pointer::read_with_modifiers` uses v2;
the existing `read` still uses v1. Pure motion coalesces only with equal masks.
All formats retain an event on failed user copy. No syscall number or right
changes. See the [approved extension](../docs/sdk/input.md#pointer-modifier-snapshots).


## File capabilities and attached IPC (M7.7)

Syscalls 91–94 are additive: `FileCapOpen(path,len,rights)`,
`FileCapOp(handle,operation,ptr,len,offset,rights)`,
`IpcSendHandle(target,handle,rights,message,len,offset)` and
`IpcReceiveHandle(message,len,sender_out,handle_out)`. Success returns a
handle, byte/entry count or zero according to the operation; errors use the
existing negative errno convention. `offset` on attached send identifies the
u32 handle field to rewrite to the receiver's actual handle ID.

`abi/transfers.abi` generates operation/relation values and XFR1 kinds/actions/
statuses into Rust and `include/dunit/transfer.h`. `libdunit::file_cap` defines
the typed wrappers, including Stat, Read, ReadDir, OpenChild, Remove, Move,
CreateChild, Write, Relation and Sync. ReadDir returns the actual entry count,
including when greater than the supplied array capacity. Relative child names
are single components; rights cannot exceed the parent capability. Move
requires WRITE on source and destination and never overwrites an existing name.
Cross-backend Move is EOPNOTSUPP. Copy is userspace policy.

Attached IPC requires recipient opt-in through syscall 94. It atomically copies
and attenuates an owned handle, attests its identity to the receiver, and rolls
back on a full queue. Legacy receive refuses an attached head without consuming
it; invalid output buffers and insufficient capacity also preserve the message.
The kernel has no clipboard owner, MIME negotiation or desktop application list.
See [M7.7 lifetime and scope](../docs/sdk/desktop-transfers.md#clipboard-and-dragdrop).

## Native Ethernet packet queue

`NetDeviceGrant` (95) inserts a nontransferable device handle into a selected
live direct child's handle table. The caller supplies its bootstrap root handle,
interface ordinal, recipient PID and device rights. The return value is a
handle number in the recipient's table. `NET_CONTROL` is bit 7 and `NET_RAW` is
bit 8. `READ` and `WRITE` constrain the operations on the resulting device and
its raw queue.

`NetQueueOpen` (96) takes a device handle and an exact 16-byte options record
containing version 1 and zero flags/reserved bytes. It returns an owner-bound
descriptor for standard read, write, poll, wait-set and close operations.
Read and write are nonblocking. Descriptors may be duplicated within the owner
and are excluded from descriptor inheritance, even with CLOEXEC clear.
An explicit spawn duplication action using a packet descriptor fails with
EBADF. File capability export is unavailable for packet queues.

Device `HandleRead` copies a 64-byte info record into a buffer of at least that
size. It reports a link bit (bit 0), MTU, the byte length of the next complete RX record (0 for none), the
MAC address and six u64 counters. `HandleWrite` requires both `WRITE` and
`NET_CONTROL` and an exact 16-byte control record with version 1. Operation 1
stops and resets an active queue. It returns EPIPE if there is no active queue.
Reserved bytes must be zero.

A packet batch consists of up to 32 whole records in at most 48 KiB. Each
record starts with a u16 Ethernet frame length, u16 zero flags and u32 reserved
zero, followed by 60–1514 frame bytes without FCS and zero padding to an
eight-byte boundary. Writes validate the entire batch before accepting a
complete prefix. A write returning EAGAIN accepted no frames. Reads return
EAGAIN for an empty queue and EFBIG for an undersized first-record buffer.
Queue revocation yields EPIPE and device failure yields EIO.

The shared codec lives in [`abi/packet`](packet). C layouts are in
[`include/dunit/packet.h`](include/dunit/packet.h).

## Outbound IPv4 application sockets

The socket ABI defines four calls. Byte lengths use native pointer-sized
arguments. Grant handles and descriptor numbers are process-local.

| Call | Native arguments | Result |
|---|---|---|
| `NetServiceRegister` (97) | device handle, options pointer, options length | Private service bridge FD |
| `NetConnectGrant` (98) | bootstrap root handle, direct child PID, scope mask | Connect handle in that child's table |
| `NetSocketOpen` (99) | connect handle, protocol, options pointer, options length | Owner-bound nonblocking socket FD |
| `NetSocketOp` (100) | FD, operation, input pointer, input length, output pointer, output length | Operation-specific result or negative errno |

`NET_CONNECT` is right bit 9 and `NET_SERVICE` is bit 10. Scope bits 0–4
authorize outbound UDP (`1`), outbound TCP (`2`), UDP bind (`4`), TCP
listen/accept (`8`), and privileged ports 1–1023 (`16`). Protocol values
1, 2 and 3 select UDP, TCP and DNS (authorized when either UDP or TCP
outbound scope is granted; TCP also allows scope `8` and UDP allows scope `4`).
Both option records are eight bytes containing little-endian version 1 in
the first two bytes and six reserved zero bytes. The native endpoint record
contains four IPv4 octets, a little-endian u16 port and a reserved zero u16.
An all-zero endpoint represents an unspecified address.

Operations use these records:

| Value | Operation | Input | Output |
|---|---|---|---|
| 1 | Connect | IPv4 endpoint | Empty |
| 2 | UDP send-to | Destination endpoint followed by datagram | Empty |
| 3 | UDP receive-from | Empty | 16-byte header followed by copied bytes |
| 4 | TCP write shutdown | Empty | Empty |
| 5 | Query error | Empty | Little-endian signed 32-bit errno |
| 6 | Query endpoints | Empty | Local and remote endpoint records |
| 7 | Bind | Local IPv4 endpoint (`0.0.0.0` or interface IP, port 0 for ephemeral) | Empty |
| 8 | Listen | Little-endian `u32` backlog | Empty |
| 9 | Accept | Empty | Optional remote IPv4 endpoint; returns new owner-bound socket FD |

The UDP receive header contains the source endpoint, a little-endian u32
length before truncation, and four reserved zero bytes. The successful byte
count includes the header. A header-only receive can report a zero-length
datagram or explicitly truncate a nonempty datagram to zero bytes.

The private bridge transports exact records with a 40-byte header and up to
4096 payload bytes. The header contains u64 socket token and process owner,
u16 kind and flags, i32 error, four IPv4 octets, u16 port, u16 protocol,
u32 payload length and four reserved zero bytes. `COMPLETE` records identify
the request kind in the flags field. All other records use zero flags.
The kernel attaches the owner and token to requests. A `CREDIT` record
reports that the application has consumed the preceding data record. DNS
sockets exchange hostname queries (up to 253 bytes) via `SEND_DATA` and
receive 4-byte IPv4 addresses via `BRIDGE_DATA` followed by `BRIDGE_COMPLETE`.

C layouts are in [`include/dunit/socket.h`](include/dunit/socket.h).
The equivalent checked codecs live in [`abi/socket`](socket).
The [N4 design](../docs/network/application-sockets-proposal.md)
describes the owner, service and authorization boundaries.

## Conformance and limits

The freestanding `/app/c_hello` uses generated headers and its own crt0 to read
stack argv/env/auxv, exercise raw write/exit and validate negative EBADF.
`tools/m6_c_abi_markers.json` checks native execution through `qemu_test.py`.
Musl/thread/fd suites validate higher adapters separately. Current support does
not imply signals, cancellation, fork/image-exec, secure entropy, dynamic linking,
full termios, POSIX ownership/accounts or a frozen previous-release fixture suite.
See [current state](../docs/architecture/current-state.md#open-gates).
