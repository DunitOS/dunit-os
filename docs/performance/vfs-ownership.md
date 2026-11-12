# OP3 — owned filesystem backends and scoped VFS access

12 November 2026. Mounts, open files and file capabilities now retain the same
`Arc<IrqSafeSpinLock<dyn FileSystem>>`. A backend outlives every outstanding open
description; all backend access, including final close, uses its lock. MemFS and
DunitFS have one authoritative owner/lock shared by namespace and direct access.
Raw filesystem pointers and the root/mounted filesystem UnsafeCell wrappers are
removed. Boot publication uses OnceCell; persistent disk format is unchanged.

The registry is protected by its own IRQ-safe lock. `get_vfs()` returns a scoped,
CPU-bound VfsGuard instead of an unrestricted static mutable reference. All
callers were migrated, including exec, descriptor inheritance/cleanup, spawn
file actions, recovery commands and driver mounting. Static asset lookup returns
only the existing immutable module-backed bytes after releasing the root lock.

Lock order is registry -> backend. Direct capability/root operations acquire
only the backend lock. Append seek/write and private-map seek/read/restore hold
one backend guard across the transaction. Capabilities no longer rely on local
IRQ masking to exclude another CPU. Unsized lock support allows a typed root
Arc and a trait-object mount Arc to refer to exactly the same allocation/lock.

## Validation

The new native ownership smoke destroys a private namespace while an ordinary
open file and a capability retain its backend, unlinks the file, writes through
the capability and reads through the open file. It checks one remaining handle
after capability drop, then uses Weak to prove backend destruction after final
close. The success marker is emitted only after these checks. FileCapability
and VirtualFileSystem Send/Sync bounds are checked by the compiler.

```sh
python3 tools/test_vfs_ownership.py --accel tcg --cpus 4
```

The driver preserves the complete existing required/forbidden budgets of nine
native scenes: filesystem mutation, file operations, lseek, fstat, dup, exclusive
create/directory rewind, native fd/private mmap/spawn pipes, thread stress and
handle/file-capability transfer. All passed on the full terminal image with
4 vCPUs, including full boot/DF assertions and the ownership smoke. The guard
host tests still pass (compile rejection cases and 80,000 concurrent updates).

DunitFS capability prepare and verify passed in two separate boots of the same
new ISO, attaching the existing test-only disk with:

```sh
python3 tools/qemu_test.py --accel tcg --cpus 4 \
  --markers-file tools/m77_file_caps_prepare_markers.json \
  --qemu-arg=-drive \
  --qemu-arg=file=/workspace/dunit-os/build/dunit-scratch.img,format=raw,if=ide \
  --timeout 360 --cmd-timeout 240 --boot-timeout 120
# Repeat with tools/m77_file_caps_verify_markers.json on the same test disk.
```

Both inode/copy/cross-filesystem/fsync and reboot/copy/identity assertions passed.
GUI smoke passed all 30 required markers on 4 vCPUs. Its terminal-image adapter
moves the unchanged original action list to command_actions for
`exec /app/gui_server`; both original marker budgets remain intact. Structured
[receipts](vfs-ownership.json) retain image hashes and VM results.

## Remaining OP3 gates

The registry lock is coarse and may span multiple I/O calls. This change makes
ownership and exclusion explicit; it does not establish throughput scaling.
Process-table ownership, cleanup outside process locks, ProcFs snapshot ordering,
wait/wakeup transactions, allocator/PMM lifetimes and remote TLB invalidation
remain necessary before AP syscall/userspace dispatch. In particular, process
cleanup -> VFS and ProcFs -> process snapshot must not become a lock cycle when
process guards are introduced. Boot-only mount/format publication is retained;
concurrent hotplug/unmount is outside this milestone. Applications still use the
UP scheduler, and no KVM or application speedup result is claimed.
