# Native threads and synchronization

`dunit::thread` owns native threads over the existing ThreadCreate/ThreadJoin/
ThreadExit ABI. `dunit::sync` provides a futex-backed `Mutex` and `Condvar`.
No syscall, capability, GUI wire format or scheduling policy changes are required.
The application scheduler remains UP: these APIs provide concurrency and lifetime
ownership, not a claim of parallel CPU execution.

```rust
extern crate alloc;
use alloc::{string::String, sync::Arc};
use dunit::{sync::Mutex, thread};

let shared = Arc::new(Mutex::new(0usize));
let other = shared.clone();
let worker = thread::spawn(move || {
    *other.lock() += 1;
    String::from("completed")
})?;
let result = worker.join(); // Result<String, thread::JoinError>
assert_eq!(*shared.lock(), 1);
```

## Ownership and failure

`spawn` accepts `FnOnce() -> T + Send + 'static`, with `T: Send + 'static`.
The default usable stack is 256 KiB; `Builder::stack_size` accepts at least 16 KiB
and rounds upward to native pages. The stack is writable/non-executable and has
an inaccessible page at either end. Each successful thread has the existing
kernel-provided per-thread static TLS instance. This is not dynamic TLS or std.

The handle owns the closure packet, typed result and stack mapping. A worker
publishes its result with Release ordering; `join` additionally obtains the native
stop acknowledgement before any backing is reclaimed. Return values do not travel
through an integer exit code. Native faults and exit-without-result return a
`JoinError` with the native kind/code. A finite completion wait also detects a
faulted worker which could not publish its completion word.

**Dropping a handle joins it.** This also drops an unclaimed result. Drop may
block: release locks needed by the worker before dropping/joining its handle.
There is intentionally no detach API, because the present ABI cannot reclaim a
thread's own running userspace stack. Self-join or an invalid native join contract
panics without freeing possibly live backing. SDK panics do not unwind: the
main execution entity exits its process, while a secondary worker exits that
worker and can return a nonzero `JoinError`. Values moved onto a worker stack
may miss their destructors on panic, fault or explicit native exit; a mutex held
by that worker can remain locked. There is no poisoning or recovery contract.

`is_finished` means the closure returned; native HAL cleanup may still be pending.
`thread_id` and `current_id` return opaque `ThreadId` values. Handles can move to
another thread, while a mutex guard cannot. Shared guards require `T: Sync`;
sharing a mutex requires `T: Send`. No raw stack pointer is exposed to apps.

## Mutex and condition variables

`Mutex::new`, `lock`, `try_lock`, `get_mut` and `into_inner` provide exclusive
payload access. The three-state lock uses Acquire/Release ordering, parks on
contention and wakes one waiter when releasing a contended lock. It is not
recursive and makes no fairness guarantee. Allocation-free synchronization
state is bounded; no unbounded waiter list is allocated by the SDK.

`Condvar::wait` releases its guard, waits and reacquires the same mutex.
`wait_while` repeats a predicate under the lock; `notify_one`/`notify_all` signal
after changing the protected state. Notifications are hints and spurious wakes
are allowed. A generation captured before unlock closes the notify/register gap.
The 32-bit generation fails explicitly on exhaustion instead of wrapping to a
stale expected value. Native futex errors inconsistent with these valid owned
objects panic rather than silently claiming successful synchronization.

```rust
let mut ready = mutex.lock();
ready = condition.wait_while(ready, |ready| !*ready);
// Producer changes *mutex.lock() then calls condition.notify_one/all().
```

## Verification

`runtime/dunit/tests/thread_sync.rs` compiles the actual SDK algorithms with a
host-only thread/VM/futex transport. Ten tests check full-width thread identities,
owned captures/results, joining
Drop, failed creation, native fault without completion, stack sizes, 8,000
contended payload updates, notify-all and 1,000 repeated condition handoffs.
Four compile-negative checks reject moving a guard, sharing a Cell guard,
non-Send captures and non-static borrowed closures. The host transport does not
claim native stack/guard/TLS behavior.

The native fixture is `tools/fixtures/sdk_threads.rs`; its contract is
`tools/sdk_threads_markers.json`. It checks eight native worker identities,
4,000 mutex updates, typed owned results, 128 condition handoffs, joining Drop
and repeated stack lifecycles through the regular SDK fixture harness.

```sh
python3 runtime/dunit/tests/check_threads.py
python3 tools/test_sdk_gui_fixture.py threads --accel tcg --cpus 4
```

No SMP scaling, asynchronous cancellation, dynamic TLS, detached/scoped threads,
timed mutex/condition waits, channels or worker pool is claimed by this slice.

Fresh TCG native checks and their source/image hashes are recorded in the
[17 November integration receipts](../verification/multitrack-2026-11-17.json).
