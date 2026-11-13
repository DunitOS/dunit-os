# OP3 — non-escaping current-process and thread access

12 November 2026. The exported current_process/current_process_mut/
current_thread_mut selectors returned unrestricted static references. Syscall,
IPC, input and fault-handling code could therefore keep process references across
later lifecycle, target lookup, backend access or user-copy stages.

The three selectors are now private to the process module. External callers use
with_current_process, with_current_process_mut and with_current_thread_mut. Their
higher-ranked callbacks permit owned results but cannot return a reference
borrowed from Process or one of its fields. The scope masks IRQs using the
existing UP convention. This is a borrowing boundary, not cross-CPU exclusion.

Migration covers PID/cwd snapshots, handle/memory/shared-frame operations,
file capabilities, attached IPC, pointer authorization, descriptor control,
anonymous/private file mappings, spawn inheritance, PTY ownership, fault PID
logging and thread-local wait-set state. Scalars return by value; cwd and owned
capabilities are cloned only where already needed. There is no whole-process
snapshot allocation for PID or permission checks.

Handle transfer takes an owned object out of the source scope before target
process mutation. Memory reads and mapping preparation return owned lengths or
addresses before user copies. Wait-set Pending boxes move in/out of the current
thread scope, preserving the distinction between a thread and its process owner.
All public wire records, ABI numbers and rights checks remain unchanged.

## Deferred descriptor notifications

FD remove/replace methods previously rescanned wait sets during a Process
mutation, entering process metadata again while a mutable process reference was
still live. They now set a pending notification bit. Both scoped mutation entry
points and with_process_mut extract the owned callback result and drain that bit,
then rescan after the process/table borrow ends, while IRQs are still masked.
Partial mutation followed by an error also flushes the notification. Non-FD
mutations do not trigger blanket rescans. Ordinary close/dup wrappers use these
entry points; pipe/backend cleanup remains outside the process mutation.

## Checks

```sh
RUSTC=rustc python3 tools/test_process_access.py
python3 tools/test_process_resources.py --cpus 4 --accel tcg
```

The host contract test compiles the actual current_access.rs module with minimal
selector/IRQ stubs. Scalar, String/Arc and moved field results compile. Returning
the Process reference, a borrowed cwd, or mutable process/thread fields fails to
compile. A callback demanding a static Process argument is also rejected. These
are lifetime checks, not host execution of privileged IRQ instructions.

Native checks retain the fifteen positive process-resource/VFS scenes and the
separate negative thread fault/join scene, with boot/DF assertions. They include
capability identity and rollback, shared fd status, private mmap, wait-set
hangup/close/reuse, IPC cancellation, PTY, spawn/pipe/condvar, and the 1,000
joinable plus 1,000 detached thread stress cycles. GUI smoke keeps all 30 markers
on four vCPUs. C pointer lock/confine/release and cursor pixels, including client
exit, pass on eight vCPUs. [Receipts](process-access.json) preserve image hashes
and outcomes. Host guard/queue and generated ABI/layout checks also pass.

## Remaining ownership work

At this checkpoint, the underlying process table still had UP-only UnsafeCell
access and private legacy current selectors. Later [dispatch ownership](dispatch-ownership.md)
removed Process borrows across HAL, and [internal task scopes](task-scopes.md)
removed those current selectors. The private mutable registry reference remains UP-only. Process
pinning, a real registry/task locking model, alias-safe internal callbacks,
atomic Ready -> Running and wait/wakeup, memory lifetimes and remote TLB shootdown
are still required before AP dispatch. IRQ masking alone protects no other CPU.
This step narrows the public kernel API and removes FD rescan reentry from its
mutation scopes; it does not claim general reentrant or SMP-safe Process access.
