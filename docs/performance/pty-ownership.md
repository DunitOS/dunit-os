# PTY ownership and deferred fd retirement

15 November 2026. OP3 safety work on top of `63faf03`; application dispatch remains UP.

The PTY table now uses a real IRQ-safe lock. Each channel transaction holds that
guard through ring or geometry access. Read, write and close notify only after
unlock; consuming slave input also notifies producers that ring space is free.
Removed channel storage is destroyed outside the table guard. Master metadata
checks run outside PTY locking, preserving EOF when a detached master's task pin
has delayed its final resource cleanup.

Slave I/O, ioctl and readiness borrow an endpoint whose constructor is private.
Read/write use one owning fd snapshot through the operation, and ioctl keeps its
endpoint through the user-memory copy. Closing a descriptor cannot destroy an
operation's channel. A final endpoint destructor checks attachment generation
identity: a new slave can attach after the old strong count reaches zero but
before its destructor acquires the table. The old destructor must not remove the
replacement's closed channel. Identity also survives a legal move out of its Arc.
Channel IDs are checked before narrowing to u32; attachment generations cannot wrap.

Fd close, dup2/install replacement, invalid/full fd allocation and terminal
binding move outgoing endpoints into a task-owned retirement batch. The common
mutation scope extracts that batch, unlocks the task, then drops it and flushes
readiness. Callback errors follow the same path. File-capability close also
retires its last owner outside the task guard. Registry callbacks remain short
state/context transactions; they do not perform descriptor operations. Nested
task locks remain forbidden. This does not make arbitrary callbacks reentrant.

The process module was split into fd transactions, registry/lifecycle APIs and
the HAL execution loop; existing public entry points and ABI numbers are intact.

`qemu_test.py --qmp-transport stdio` uses nonblocking local pipes with the same
QMP framing as the default Unix socket transport. Partial writes and fragmented
replies have deadlines; asynchronous events cannot extend command timeout.
QEMU stderr is retained in each work directory. This lets the existing harness
run on a host that disallows socket creation. No alternative QEMU runner was added.

Verification uses actual modules, not copied algorithms:

- PTY host tests pass 100,000 ordered bytes in each direction on four concurrent
  host threads, reenter table access from metadata/wake hooks, check capacity,
  geometry, inherited ownership, EOF and replacement-before-old-destructor.
- Task-access tests retain their compile-negative lifetime checks and 8,000
  concurrent mutations, and exercise retired destructors reentering the same task
  after callback error or a missing lookup.
- Native smoke arms real generation-tagged blocked tasks and verifies readiness
  after master/slave read/write, final fd drops, fd errors and empty stdio binding.
  Weak owners, channel count and backend handle count verify final release.
- Resource, fd/PTY argv, wait-set, pipe, IPC and thread lifecycle regressions pass
  on BIOS/TCG with four hardware CPUs. Full boot passes on one and eight CPUs.
  GUI passes on four CPUs, including resize, client recycle and the cooked dsh path.
- Production compilation and the separate production boot contract verify the
  foundation runtime without boot-smoke probes. ABI and the affected host
  execution/parking/IPC/user-memory/QMP contracts remain green.

The first full boot attempt failed only because its old clock budget still
required PIT as the clocksource. The foundation kernel selects q35 HPET and PIT
timer events. The budget now asserts that actual selection and retains every
other boot requirement and forbidden marker. The failed receipt is preserved.

[Structured receipts](pty-ownership.json) record source state, image hashes,
results and the failed attempt. These are correctness checks, not a latency or
scaling claim. APs perform hardware/allocator/TLB probes and then park;
`scheduler=up dispatch_cpus=1`. Console/stdin ownership, remote cancel/stop,
interruptible syscalls, the full driver/heap lock order and remote CPL3/shared-VM
stress remain open. KVM and UEFI were not exercised in this change.

Relevant native markers:

```text
[PTY-LOCK] owning I/O, read/write wake, hangup and drain OK
[FD-RETIREMENT] final PTY/cap drops, fd errors and wake reentry after unlock OK
[PROCESS-RESOURCES] detach, dup, rollback, kill EOF/cap close before wait OK
[THREAD-TEST] OK
```
