# Poll, wait-set and attached IPC guard scopes

Poll copies and validates up to 1,024 input records (8 KiB) before the park
transaction. Wait-set prepares its 64-entry request (2 KiB) and result storage
before registration. Both take their notification checkpoint before readiness
probes. A notification between the probe and registration invalidates the
checkpoint; the caller retries instead of sleeping on stale readiness. Wait-set
also retains descriptor generation across restart and retires discarded request
storage outside the task scope.

Attached IPC send copies up to 256 payload bytes before entering the capability
and publication transaction. Failed publication rolls back the inserted handle.
Wake notification and payload retirement run after the transaction. Receive
keeps its preflight, copy and consume atomic for up to 264 output bytes. Poll
also retains atomic output mapping and copy for up to 8 KiB.

The host regression harness compiles the actual syscall source against test
fixtures and checks guard placement, notification races, descriptor reuse,
receive faults and publication rollback. See
[`tools/test_syscall_wait_scopes.py`](../../tools/test_syscall_wait_scopes.py).
These checks establish structural bounds and ordering. End-to-end IRQ latency,
other copyout scopes, heap behavior in capability and queue operations,
and remote application dispatch remain open.
