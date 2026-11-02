# Pointer modifier snapshots — proposed compatible ReadPointerEvent extension

The approved gui-v1 lifecycle proposal explicitly requires a separate decision
if pointer-queue public semantics must change. That condition is now confirmed:
keyboard and pointer have independent queues, and the compositor drains a queued
pointer record before draining keyboard records. Neither queue shares a timestamp
or sequence. Sampling current/last keyboard modifiers can therefore apply the
state from a later release to an earlier click. Draining keys first merely changes
the race. This must not be represented as correct pointer selection.

Recommended contract (not implemented before approval):

- Keep syscall 90, its three existing arguments (INPUT_MASTER handle, output,
  length), privilege checks, EAGAIN and transactional EFAULT behavior.
- Preserve length 32 as exact existing v1 bytes. Reserved bytes remain zero.
  Existing kernels/clients and v1 decoders are unchanged.
- Add an explicitly requested length-40 v2 record. Bytes 0..32 retain v1 layout;
  bytes 32..36 hold a u32 Shift/Ctrl/Alt/Super mask (bits 0..3), sampled in the
  input IRQ critical section when the pointer sample is enqueued. Bytes 36..40
  are reserved zero. Unknown modifier bits fail decoding.
- The existing key-event ABI stays unchanged. The kernel exposes device modifier
  facts, while selection/focus policy remains in userspace. No new syscall number,
  rights, manifest format, or event timestamps are introduced.
- Queue entries retain the snapshot even if keys change before the reader runs.
  Motion must not coalesce across a modifier change. Overflow/handover keep the
  explicit LOST_BEFORE capture-reset semantics; an ownership reset snapshots the
  current modifier state too.
- New libdunit `read_with_modifiers` requests 40 bytes; old `read` requests 32.
  An old kernel returns EINVAL for the new length, rather than inventing modifiers.
  The compositor must not silently approximate missing pointer snapshots.
- Tests: exact v1 codec preservation; v2 validation; failed-copy retention; queue
  modifier ordering/coalescing; native held-Shift/Ctrl click with release before
  reader resumes; denied INPUT_MASTER; existing pointer queue and GUI regressions.

Alternative: a unified timestamped/ordered keyboard+pointer stream. This is a
larger ABI and delivery change, unnecessary for a per-pointer modifier snapshot.
The length-selected v2 is recommended for the narrow behavior required here.
