# OP3 — detached process teardown and descriptor inheritance

12 November 2026. External resource cleanup is now separated from process/table
mutation. `process/resources.rs` owns the detached fd and VM/capability batches.
Taking a batch empties the corresponding fields; releasing or dropping it closes
resources without borrowing the original Process. Batch Drop provides cleanup
on failure as well as the successful path.

## Behavior and lifetime changes

Normal exit detaches fd ownership inside with_process_mut and releases it after
that call returns. After switching away from the address space, VM mappings,
shared-frame references and capabilities are detached and released separately.
Kill publishes the dead status and extracts ownership in an IRQ-masked section,
then performs scheduler/IPC cleanup and releases resources outside that section.
A killed process now closes fds and pipe writers before its parent calls wait;
previously those fds could remain live until reap, postponing EOF indefinitely.

Wait/reap, failed spawn autoreap, prepared-child cleanup, owned-thread cleanup,
join, thread rollback and process-record replacement move owned objects out of
the table before their destructors run. Process Drop drains any residual fd/VM
ownership so an error path cannot silently bypass final close. IRQ masking does
not substitute for a future cross-CPU process-table ownership model.

Spawn inheritance first constructs a complete owned descriptor set outside the
child mutation. It retains each distinct VFS description once and each pipe end
per descriptor. Mutation only swaps the fd maps; the displaced set is released
afterward. Partial construction or a missing child rolls back via batch Drop.
Dup counts remain per process fd, while VFS references remain per distinct open
description. Shared status flags and PTY endpoints retain their Arc identity.

Global fd lookup returns an owned FdEntry snapshot rather than a static reference
into a mutable table. Native fd and terminal callers consume that snapshot.
This removes escaped fd-table references; it is not yet a cross-CPU VFS lease.

## Validation

```sh
python3 tools/test_process_resources.py --cpus 4 --accel tcg
```

The native boot smoke verifies partial inheritance rollback, dup accounting,
empty source maps after detachment and final close after the last inherited set.
It also creates a prepared child holding a VFS file, a file capability and a pipe
writer, kills it and checks EOF, descriptor invalidation and restored backend
handle count **before wait**. Wait then returns the checked exit code and removes
the child. Only successful assertions produce the PROCESS-RESOURCES OK marker.

The driver retains all budgets of the nine VFS/handle scenes, adding reap order,
wait sets, PTY argv, native spawn, pipe and condvar tests: fifteen positive scenes.
The thread lifecycle negative fixture runs in a separate VM with its original
budget: it intentionally page-faults a thread and verifies fault-to-join delivery.
Keeping it separate preserves the positive scenes' no-user-fault budget.
Both groups retain full boot/DF/IRQ checks. Native thread stress includes 1,000
joinable TLS/errno/allocation cycles and 1,000 detached auto-reaps.

GUI smoke on 4 vCPUs retains all 30 markers; the terminal-image adapter preserves
the original actions and both marker budgets. Native C pointer tests on 8 vCPUs
check lock/confine/release and exact cursor pixels, including client exit.
[Structured receipts](process-resources.json) record the image and VM outcomes.
The existing host guard tests and generated ABI/layout checks also passed.

## Remaining gates

The process table still uses UP-only mutable access, current-process references
are not yet guarded/pinned, and scheduler/wait-queue ownership remains coupled.
This change prepares their locking boundaries; it does not enable AP syscalls or
parallel application dispatch. ProcFs snapshot ordering, atomic block/wake,
address-space lifetime, allocator/PMM and TLB shootdown still need implementation
and cross-CPU stress. No public ABI, rights model or persistent format changes.
