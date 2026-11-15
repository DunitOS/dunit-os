# Storage & Filesystems

This article documents the Virtual File System (VFS), ramdisk packaging (Initrd), in-memory filesystem (MemFS), and persistent on-disk filesystem (**DunitFS v2**) in **Dunit OS**.

---

## 1. Virtual File System (VFS) Architecture

The VFS (`kernel/src/fs/vfs.rs`) provides a unified, POSIX-compatible filesystem abstraction:

```text
+-------------------------------------------------------------------------+
|                              Userspace                                  |
|            fopen() / fread() / open() / read() / readdir()              |
+------------------------------------+------------------------------------+
                                     |
                                     v (Syscalls: 3=read, 4=write, 5=open, 6=close)
+-------------------------------------------------------------------------+
|                        VFS Layer (kernel/src/fs/)                       |
|   - File Descriptor Table: Per-process array mapping FDs to Inodes      |
|   - Standard Streams: FD 0 (stdin), FD 1 (stdout), FD 2 (stderr)        |
|   - Path Resolution: Absolute & relative path parsing, CWD resolution   |
|   - Mount Table: Routing paths to filesystem drivers                    |
+------------------------------------+------------------------------------+
                   |                                  |
                   v                                  v
+------------------------------------+ +----------------------------------+
|           MemFS (Root /)           | |         DunitFS v2 (Disk)        |
|  In-memory dynamic tree and files  | |  Superblock, Inodes, Extents     |
+------------------------------------+ +----------------------------------+
```

### Standard File Descriptors
Every process inherits three standard descriptors:
* `0` (`stdin`): Standard input stream (keyboard or pipe).
* `1` (`stdout`): Standard output stream (connected to terminal/serial log).
* `2` (`stderr`): Standard error output stream.
* `3+`: Dynamically allocated descriptors returned by `open()`.

---

## 2. In-Memory Filesystem (MemFS)

MemFS (`kernel/src/fs/memfs.rs`) serves as the default root filesystem (`/`):
* **Storage**: Inodes and file contents reside entirely in kernel-managed RAM.
* **Nodes**: Supports regular files, directories, and symbolic references.
* **Characteristics**: Fast access, zero hardware seek latency, and fully volatile across reboots.

---

## 3. Initial Ramdisk (Initrd)

To deliver application binaries, icons, fonts, and configuration files to the live system without requiring disk drivers during early boot, Dunit OS uses an initial ramdisk (`initrd.img`):

1. **Build Process**: The build script (`build_iso.sh` or `build_bundle_iso.sh`) packs all files from `userspace/`, `assets/`, and `overlay/` into a single archive:
   ```bash
   [INITRD] packed 1126 files -> /home/tuxpc/dunit-os/build/initrd.img (79905302 bytes)
   ```
2. **Boot Handoff**: Limine loads `initrd.img` into physical memory and passes the base address to the kernel.
3. **Extraction**: During kernel initialization (`initrd::init()`), the archive header is unpacked and all 1,126+ files and directories are populated into MemFS at `/`.

---

## 4. Persistent On-Disk Storage (DunitFS v2)

For physical disk partitions and virtual drive images, Dunit OS implements **DunitFS v2**:

### On-Disk Layout

```text
+----------------+----------------+----------------+----------------+-----------------+
|   Superblock   |  Block Bitmap  |  Inode Bitmap  |  Inode Table   |   Data Extents  |
|    Block 0     |    Block 1     |    Block 2     |  Blocks 3..64  |  Blocks 65..End |
+----------------+----------------+----------------+----------------+-----------------+
```

* **Superblock**: Stores filesystem magic bytes (`0x44554E49` / "DUNI"), total block count, free block count, inode count, and block size (typically 4096 bytes).
* **Block & Inode Bitmaps**: Track allocated and available disk blocks and inode numbers.
* **Inode Table**: Stores file metadata (file size, permissions, creation/modification timestamps, and direct/indirect block extent pointers).
* **Extent Allocation**: Files allocate contiguous block extents to maximize sequential read/write throughput and minimize fragmentation on solid-state and spinning disks.

### Filesystem Consistency (`fsck`)
Dunit OS provides an automated consistency checker (`tools/m5_fsck.py` / `tools/m5_dunitfs_v2_markers.json`) that validates:
* Inode link counts.
* Bitmap allocation parity against referenced data blocks.
* Directory entry reachability and avoidance of circular references.

