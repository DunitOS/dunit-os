# Futex admission lock order — OP3

13 November 2026. The next wait-queue prerequisite after the
[wake callback change](wait-queue.md). Application dispatch remains UP.

## Implementation

Previously `futex_wait` locked WAIT_QUEUE, then looked up the owner/process,
locked AddressSpace, translated the word, compared it and looked up the calling
task again. Introducing task/registry locks there would invert the existing
process → queue blocking order.

The new path obtains an owned AddressSpace Arc during a short registry lookup,
then selects task state, locks AddressSpace and resolves the word before locking
WAIT_QUEUE. The final Acquire load, task Blocked transition and registration
remain under the queue lock. No registry lookup or mapping lock acquisition occurs
under it. This preserves compare/register serialization with wake selection;
merely moving the word comparison before the lock would lose that protection.

The private `UserWord` view checks alignment and PRESENT|USER, borrows the locked
AddressSpace and exposes only an owned value from its Acquire load. It cannot
outlive that borrow or expose a physical pointer/Atomic reference. Clear-child-tid
additionally requires WRITABLE and stores zero with Release, then releases the
mapping guard before notifying futex waiters. Page-end aligned words are valid;
PROT_NONE and unmapped words are rejected. Old plain unaligned word accesses are
removed. Futex lifecycle/admission is extracted from process/mod.rs; dispatch
and futex share the owned AddressSpace lookup rather than duplicating it.

## Validation

```sh
python3 tools/test_user_word.py
python3 tools/test_wait_queue.py
python3 tools/test_scheduler_queue.py
python3 tools/test_futex_admission.py --accel tcg --cpus 4
python3 tools/test_process_resources.py --reuse-image --accel tcg --cpus 4
```

The host test compiles the actual futex, word-view, wait-queue and SpinLock code
with explicit registry/VMM/IRQ/HAL test doubles. Registry and mapping lookup
assert that WAIT_QUEUE is unlocked; context capture asserts both mapping and
queue locks are held. It checks mapping permissions, alignment, mismatch with
no registration, park/wake and writable-only clearing. A compile-negative case
must reject returning a UserWord whose mapping guard has ended (E0515).
This is a deterministic ordering/lifetime test, not a cross-CPU task test.

The native Rust fixture preserves mismatch, timeout, wake and join assertions.
It adds raw-syscall EINVAL for unaligned input, EAGAIN on a read-only page-end
word, EFAULT for PROT_NONE/unmapped pages, exactly one wake out of two waiters,
then exactly one remaining wake and zero stale registrations. Invalid-pointer
checks never form invalid Rust references. The fixture's bounded-wake counts
also require both waiters to have reached the queue.

The native combined budget retains every original marker from musl mutex/join,
100-generation three-thread barrier, pipe, directory and condvar fixtures.
The focused fixture passed on 1 vCPU; the six-command combined budget passed
on 4 vCPUs (20 required / 16 forbidden entries). All 15 positive resource scenes
and the separate negative thread lifecycle fixture passed on 4 vCPUs using the
same full ISO. Join and detached checks retain 1,000 cycles each. Production and
boot-smoke kernel checks, generated ABI/CPU layout checks and host tests pass.
VM provenance, image hashes and results are in [the receipt](futex-admission.json).
Run wall times in that receipt describe whole tests, not per-wake latency;
no performance or scaling improvement is inferred.
No test budget was weakened.

## Remaining gates

No actual process registry/task locks are added here. The task borrow and state
transition still rely on the UP IRQ boundary; a selected waiter can race remote
cancellation/dispatch once AP scheduling is introduced. Atomic task execution
claims, a cross-CPU wait/block/cancel protocol and remote notifications remain
open. AddressSpace ownership/locking does not solve shared-frame bookkeeping,
active CPU tracking or acknowledged TLB shootdown before frame reuse. Allocating
registration storage still uses the existing Vec/allocator path under the queue
lock and needs the full heap/PMM lock-order audit. APs remain parked; TCG CPU
counts do not demonstrate parallel application execution or speedup.
