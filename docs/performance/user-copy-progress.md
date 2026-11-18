# Page-bounded owned source copies

Bulk user input previously grew a Vec and read up to the 64 KiB ABI copy limit
inside one address-space IRQ-safe guard. Fixed-length strings also performed
NUL/UTF-8 validation and owned-output retirement inside that scope.

The input helper now validates range/cap, allocates and initializes owned output
before acquiring a mapping guard, and copies through page-aligned scopes of at
most 4096 bytes. Each chunk reacquires the active address-space owner and validates
its current mapping/permissions. No source pointer or unlocked view escapes.
Only a fully copied owned result reaches the caller; errors drop output after the
mapping guard ends. Strings check NUL and UTF-8 after the copy, preserving range,
length, zero-size and invalid-input errors. This is not a transactional snapshot
of concurrently writable userspace memory.

The actual-source host test measures 16 separate pins for an aligned 64 KiB input
and three for an unaligned 4098-byte input. A checking global allocator aborts
allocation/reallocation/free under a mapped guard. Tests revoke permissions and
free the backing before chunk two, validate EFAULT without another dereference,
and cover UTF-8 spanning a page, early NUL with a later mapping fault, invalid
UTF-8, existing copyout permissions and nonescaping/non-Send views.

Copyout remains unchanged, preserving its full-range validation before writes.
The helper's change does not bound heap allocator critical sections, callers'
outer InterruptGuard scopes or arbitrary other with_mapping users. Native fd
poll, wait-set and attached-IPC callers retain outer guards. No universal elapsed
IRQ-off bound, PIT progress or FPS improvement is inferred for these helpers.
The [follow-up receipts](../verification/multitrack-followup-2026-11-18.json)
separate host scope measurements from native boot/SDK/IPC/filesystem regressions.
