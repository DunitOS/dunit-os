use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::sync::Arc;
use crate::sync::{IrqSafeSpinLock, OnceCell};
pub type Backend = Arc<IrqSafeSpinLock<dyn FileSystem>>;
use core::ops::{Deref, DerefMut};
use core::fmt;

use super::memfs::MemFs;

// Applications (/app/*), desktop assets (/assets/**) and the DWM system config
// tree (/system/share/dwm/**) are NO LONGER embedded in the kernel via
// `include_bytes!`. They ship in a Limine-loaded initrd archive (tools/
// pack_initrd.py -> kernel/src/initrd.rs); `init()` below populates the root
// MemFS from `crate::initrd::get_initrd().entries()`, so the kernel knows no app
// or asset name at compile time — every path comes from the archive at runtime.

pub type FileDescriptor = u32;
pub type FileHandle = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenFlags {
    bits: u32,
}

impl OpenFlags {
    pub const READ: Self = Self { bits: 1 << 0 };
    pub const WRITE: Self = Self { bits: 1 << 1 };
    pub const CREATE: Self = Self { bits: 1 << 2 };
    pub const TRUNC: Self = Self { bits: 1 << 3 };
    pub const APPEND: Self = Self { bits: 1 << 4 };
    /// Exclusive create: with CREATE, fail (AlreadyExists -> EEXIST) if the path
    /// already exists. Backs POSIX O_EXCL (atomic create used by mkstemp/tcc).
    pub const EXCL: Self = Self { bits: 1 << 5 };
    pub const READ_WRITE: Self = Self {
        bits: Self::READ.bits | Self::WRITE.bits,
    };

    const VALID_BITS: u32 = Self::READ.bits
        | Self::WRITE.bits
        | Self::CREATE.bits
        | Self::TRUNC.bits
        | Self::APPEND.bits
        | Self::EXCL.bits;

    pub const fn from_bits(bits: u32) -> Self {
        Self { bits }
    }

    pub const fn bits(self) -> u32 {
        self.bits
    }

    pub const fn contains(self, other: Self) -> bool {
        (self.bits & other.bits) == other.bits
    }

    pub const fn can_read(self) -> bool {
        self.contains(Self::READ)
    }

    pub const fn can_write(self) -> bool {
        self.contains(Self::WRITE)
    }

    pub const fn create(self) -> bool {
        self.contains(Self::CREATE)
    }

    pub const fn trunc(self) -> bool {
        self.contains(Self::TRUNC)
    }

    pub const fn append(self) -> bool {
        self.contains(Self::APPEND)
    }

    pub const fn is_exclusive(self) -> bool {
        self.contains(Self::EXCL)
    }

    pub const fn is_valid(self) -> bool {
        let has_unknown = (self.bits & !Self::VALID_BITS) != 0;
        let has_access_mode = self.can_read() || self.can_write();
        let write_only_modifiers_ok = (!self.trunc() && !self.append()) || self.can_write();
        // EXCL is only meaningful together with CREATE; reject it alone rather
        // than silently ignore it (M6.1: no quiet no-op).
        let excl_needs_create = !self.is_exclusive() || self.create();

        !has_unknown && has_access_mode && write_only_modifiers_ok && excl_needs_create
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    File,
    Directory,
    Device,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirEntry {
    name: [u8; 64],
    name_len: usize,
    pub file_type: FileType,
}

impl DirEntry {
    pub const fn empty() -> Self {
        Self {
            name: [0; 64],
            name_len: 0,
            file_type: FileType::File,
        }
    }

    pub fn new(name: &str, file_type: FileType) -> Self {
        let mut entry = Self {
            name: [0; 64],
            name_len: 0,
            file_type,
        };
        let bytes = name.as_bytes();
        let len = bytes.len().min(entry.name.len());
        let mut idx = 0;
        while idx < len {
            entry.name[idx] = bytes[idx];
            idx += 1;
        }
        entry.name_len = len;
        entry
    }

    pub fn name(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("<invalid>")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStat {
    pub file_type: FileType,
    pub size: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfsError {
    NotFound,
    PermissionDenied,
    InvalidDescriptor,
    AlreadyExists,
    NotADirectory,
    IsADirectory,
    InvalidPath,
    Unsupported,
    IoError,
    DirectoryNotEmpty,
    FileTooBig,
}

impl fmt::Display for VfsError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            VfsError::NotFound => write!(f, "File not found"),
            VfsError::PermissionDenied => write!(f, "Permission denied"),
            VfsError::InvalidDescriptor => write!(f, "Invalid file descriptor"),
            VfsError::AlreadyExists => write!(f, "File already exists"),
            VfsError::NotADirectory => write!(f, "Not a directory"),
            VfsError::IsADirectory => write!(f, "Is a directory"),
            VfsError::InvalidPath => write!(f, "Invalid path"),
            VfsError::Unsupported => write!(f, "Operation unsupported"),
            VfsError::IoError => write!(f, "I/O error"),
            VfsError::DirectoryNotEmpty => write!(f, "Directory not empty"),
            VfsError::FileTooBig => write!(f, "File too large"),
        }
    }
}

pub type Result<T> = core::result::Result<T, VfsError>;

pub trait FileSystem: Send {
    /// Capability anchors are backend-owned open descriptions, including directories.
    /// Unsupported backends must fail rather than resolving an ambient path later.
    fn cap_relation(&mut self,_source:FileHandle,_destination:FileHandle)->Result<dunit_file_cap_abi::Relation>{Err(VfsError::Unsupported)}
    fn cap_open(&mut self, _path: &str, _flags: OpenFlags) -> Result<FileHandle> { Err(VfsError::Unsupported) }
    fn cap_child(&mut self, _parent: FileHandle, _path: &str, _flags: OpenFlags) -> Result<FileHandle> { Err(VfsError::Unsupported) }
    fn cap_readdir(&mut self, _handle: FileHandle) -> Result<Vec<DirEntry>> { Err(VfsError::Unsupported) }
    fn cap_remove(&mut self, _handle: FileHandle) -> Result<()> { Err(VfsError::Unsupported) }
    fn cap_move_to(&mut self, _source: FileHandle, _destination: FileHandle, _leaf: &str) -> Result<()> { Err(VfsError::Unsupported) }
    fn cap_create_child(&mut self, _parent: FileHandle, _leaf: &str, _kind: FileType, _flags: OpenFlags) -> Result<FileHandle> { Err(VfsError::Unsupported) }
    fn open(&mut self, path: &str, flags: OpenFlags) -> Result<FileHandle>;
    fn read(&mut self, handle: FileHandle, buf: &mut [u8]) -> Result<usize>;
    fn write(&mut self, handle: FileHandle, buf: &[u8]) -> Result<usize>;
    fn close(&mut self, handle: FileHandle) -> Result<()>;
    fn readdir(&mut self, path: &str) -> Result<Vec<DirEntry>>;
    fn readdir_into(&mut self, path: &str, entries: &mut [DirEntry]) -> Result<usize>;
    fn create(&mut self, path: &str) -> Result<()>;
    fn mkdir(&mut self, path: &str) -> Result<()>;
    fn remove(&mut self, path: &str) -> Result<()>;
    /// Remove an empty directory. Filesystems that cannot support it inherit the
    /// default (Unsupported).
    fn remove_dir(&mut self, _path: &str) -> Result<()> {
        Err(VfsError::Unsupported)
    }
    fn truncate(&mut self, path: &str) -> Result<()>;
    fn stat(&mut self, path: &str) -> Result<FileStat>;

    /// Rename/move an entry within this filesystem. Filesystems that cannot
    /// support it (device/proc/read-only backends) inherit this default.
    fn rename(&mut self, _old: &str, _new: &str) -> Result<()> {
        Err(VfsError::Unsupported)
    }

    /// Reposition the read/write offset of an open handle. `whence`: 0=SET,
    /// 1=CUR, 2=END (POSIX lseek). Returns the resulting absolute offset.
    /// Filesystems that cannot support it inherit the default (Unsupported).
    fn seek(&mut self, _handle: FileHandle, _offset: i64, _whence: u32) -> Result<u64> {
        Err(VfsError::Unsupported)
    }

    /// Stat an open handle (fstat). Filesystems that cannot support it inherit
    /// the default (Unsupported).
    fn stat_handle(&mut self, _handle: FileHandle) -> Result<FileStat> {
        Err(VfsError::Unsupported)
    }

    /// Flush any buffered metadata/data for an open handle to stable storage.
    /// In-memory and mechanism filesystems have nothing to flush and inherit
    /// this no-op; persistent backends (DunitFS) override it to commit.
    fn fsync(&mut self, _handle: FileHandle) -> Result<()> {
        Ok(())
    }

    /// Set the length of an open handle's file to `len` (ftruncate): grow with
    /// zero bytes or shrink, without moving the handle's read/write offset. The
    /// handle must be writable. Filesystems that cannot support it inherit the
    /// default (Unsupported).
    fn ftruncate_handle(&mut self, _handle: FileHandle, _len: u64) -> Result<()> {
        Err(VfsError::Unsupported)
    }
}

pub struct OpenFile {
    fs: Backend,
    handle: FileHandle,
    flags: OpenFlags,
    refs: usize,
}

impl OpenFile {
    pub fn new(fs: Backend, handle: FileHandle, flags: OpenFlags) -> Self {
        Self { fs, handle, flags, refs: 1 }
    }

    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.fs.lock().read(self.handle, buf)
    }

    pub fn write(&mut self, buf: &[u8]) -> Result<usize> {
        let mut fs = self.fs.lock();
        if self.flags.append() { fs.seek(self.handle, 0, 2)?; }
        fs.write(self.handle, buf)
    }

    pub fn seek(&mut self, offset: i64, whence: u32) -> Result<u64> {
        self.fs.lock().seek(self.handle, offset, whence)
    }

    pub fn stat(&mut self) -> Result<FileStat> {
        self.fs.lock().stat_handle(self.handle)
    }

    pub fn fsync(&mut self) -> Result<()> {
        self.fs.lock().fsync(self.handle)
    }

    pub fn ftruncate(&mut self, len: u64) -> Result<()> {
        self.fs.lock().ftruncate_handle(self.handle, len)
    }
}

impl Drop for OpenFile {
    fn drop(&mut self) {
        let _ = self.fs.lock().close(self.handle);
    }
}

pub struct VirtualFileSystem {
    root_fs: Option<Backend>,
    mounts: Vec<MountPoint>,
    open_files: BTreeMap<FileDescriptor, OpenFile>,
    next_fd: FileDescriptor,
}

struct MountPoint {
    path: String,
    fs: Backend,
}

impl VirtualFileSystem {
    pub fn open_capability(&mut self, cwd: &str, path: &str, flags: OpenFlags) -> Result<alloc::sync::Arc<super::file_cap::FileCapability>> {
        let mut path_buffer = [0; 256];
        let (fs, relative) = self.resolve_path(path, cwd, &mut path_buffer)?;
        let handle = fs.lock().cap_open(relative, flags)?;
        Ok(alloc::sync::Arc::new(super::file_cap::FileCapability::new(fs, handle)))
    }
    pub fn new() -> Self {
        Self {
            root_fs: None,
            mounts: Vec::new(),
            open_files: BTreeMap::new(),
            next_fd: 3,
        }
    }

    pub fn mount(&mut self, path: &str, fs: Backend) -> Result<()> {
        if path == "/" {
            self.root_fs = Some(fs);
            return Ok(());
        }
        if !path.starts_with('/') || path.ends_with('/') || path.len() > 255 {
            return Err(VfsError::InvalidPath);
        }
        if self.mounts.iter().any(|mount| mount.path == path) {
            return Err(VfsError::AlreadyExists);
        }
        self.mounts.push(MountPoint {
            path: String::from(path),
            fs,
        });
        Ok(())
    }

    fn resolve_path<'a>(
        &self,
        path: &str,
        cwd: &str,
        buffer: &'a mut [u8; 256],
    ) -> Result<(Backend, &'a str)> {
        let normalized = normalize_path_into(path, cwd, buffer)?;
        let mut selected: Option<&MountPoint> = None;
        for mount in &self.mounts {
            let exact = normalized == mount.path;
            let child = normalized.starts_with(&mount.path)
                && normalized.as_bytes().get(mount.path.len()) == Some(&b'/');
            if (exact || child)
                && selected
                    .map(|current| mount.path.len() > current.path.len())
                    .unwrap_or(true)
            {
                selected = Some(mount);
            }
        }
        let (fs, relative) = if let Some(mount) = selected {
            let relative = normalized[mount.path.len()..].trim_start_matches('/');
            (mount.fs.clone(), relative)
        } else {
            (
                self.root_fs.clone().ok_or(VfsError::NotFound)?,
                normalized.trim_start_matches('/'),
            )
        };

        Ok((fs, relative))
    }

    pub fn open_at(&mut self, cwd: &str, path: &str, flags: OpenFlags) -> Result<FileDescriptor> {
        let mut path_buffer = [0; 256];
        let (fs, relative_path) = self.resolve_path(path, cwd, &mut path_buffer)?;
        let fs_flags = OpenFlags::from_bits(flags.bits() & !OpenFlags::APPEND.bits());
        let handle = fs.lock().open(relative_path, fs_flags)?;

        let fd = self.next_fd;
        self.next_fd += 1;
        self.open_files.insert(fd, OpenFile::new(fs, handle, flags));

        Ok(fd)
    }

    pub fn open(&mut self, path: &str, flags: OpenFlags) -> Result<FileDescriptor> {
        self.open_at("/", path, flags)
    }

    pub fn read(&mut self, fd: FileDescriptor, buf: &mut [u8]) -> Result<usize> {
        let open_file = self
            .open_files
            .get_mut(&fd)
            .ok_or(VfsError::InvalidDescriptor)?;
        open_file.read(buf)
    }

    pub fn write(&mut self, fd: FileDescriptor, buf: &[u8]) -> Result<usize> {
        let open_file = self
            .open_files
            .get_mut(&fd)
            .ok_or(VfsError::InvalidDescriptor)?;
        open_file.write(buf)
    }

    pub fn seek(&mut self, fd: FileDescriptor, offset: i64, whence: u32) -> Result<u64> {
        let open_file = self
            .open_files
            .get_mut(&fd)
            .ok_or(VfsError::InvalidDescriptor)?;
        open_file.seek(offset, whence)
    }

    pub fn fstat(&mut self, fd: FileDescriptor) -> Result<FileStat> {
        let open_file = self
            .open_files
            .get_mut(&fd)
            .ok_or(VfsError::InvalidDescriptor)?;
        open_file.stat()
    }

    /// Each process owns one reference per distinct open description; dup fds
    /// in that process are counted by its fd table. Child inheritance retains.
    pub fn retain(&mut self, fd: FileDescriptor) -> Result<()> {
        let file = self.open_files.get_mut(&fd).ok_or(VfsError::InvalidDescriptor)?;
        file.refs = file.refs.checked_add(1).ok_or(VfsError::IoError)?;
        Ok(())
    }
    pub fn close(&mut self, fd: FileDescriptor) -> Result<()> {
        let file = self.open_files.get_mut(&fd).ok_or(VfsError::InvalidDescriptor)?;
        file.refs -= 1;
        if file.refs == 0 { self.open_files.remove(&fd); }
        Ok(())
    }
    pub fn set_append(&mut self, fd: FileDescriptor, append: bool) -> Result<()> {
        let file = self.open_files.get_mut(&fd).ok_or(VfsError::InvalidDescriptor)?;
        let bits = file.flags.bits() & !OpenFlags::APPEND.bits();
        file.flags = OpenFlags::from_bits(bits | if append { OpenFlags::APPEND.bits() } else { 0 });
        Ok(())
    }
    /// Eager private-map input. Save and restore the shared offset even when
    /// the backing filesystem reports an error; no userspace-visible seek.
    pub fn read_at(&mut self, fd: FileDescriptor, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if offset > i64::MAX as u64 { return Err(VfsError::InvalidPath); }
        let file = self.open_files.get_mut(&fd).ok_or(VfsError::InvalidDescriptor)?;
        let mut fs = file.fs.lock();
        let saved = fs.seek(file.handle, 0, 1)?;
        fs.seek(file.handle, offset as i64, 0)?;
        let result = fs.read(file.handle, buf);
        fs.seek(file.handle, saved as i64, 0)?;
        result
    }
    pub fn fsync(&mut self, fd: FileDescriptor) -> Result<()> {
        let open_file = self
            .open_files
            .get_mut(&fd)
            .ok_or(VfsError::InvalidDescriptor)?;
        open_file.fsync()
    }

    pub fn ftruncate(&mut self, fd: FileDescriptor, len: u64) -> Result<()> {
        let open_file = self
            .open_files
            .get_mut(&fd)
            .ok_or(VfsError::InvalidDescriptor)?;
        open_file.ftruncate(len)
    }

    pub fn open_file_count(&self) -> usize {
        self.open_files.len()
    }

    pub fn readdir_at(&mut self, cwd: &str, path: &str) -> Result<Vec<DirEntry>> {
        let mut path_buffer = [0; 256];
        let (fs, relative_path) = self.resolve_path(path, cwd, &mut path_buffer)?;
        { let result = fs.lock().readdir(relative_path); result }
    }

    pub fn readdir_into_at(
        &mut self,
        cwd: &str,
        path: &str,
        entries: &mut [DirEntry],
    ) -> Result<usize> {
        let mut path_buffer = [0; 256];
        let (fs, relative_path) = self.resolve_path(path, cwd, &mut path_buffer)?;
        { let result = fs.lock().readdir_into(relative_path, entries); result }
    }

    pub fn create_at(&mut self, cwd: &str, path: &str) -> Result<()> {
        let mut path_buffer = [0; 256];
        let (fs, relative_path) = self.resolve_path(path, cwd, &mut path_buffer)?;
        { let result = fs.lock().create(relative_path); result }
    }

    pub fn mkdir_at(&mut self, cwd: &str, path: &str) -> Result<()> {
        let mut path_buffer = [0; 256];
        let (fs, relative_path) = self.resolve_path(path, cwd, &mut path_buffer)?;
        { let result = fs.lock().mkdir(relative_path); result }
    }

    pub fn remove_at(&mut self, cwd: &str, path: &str) -> Result<()> {
        let mut path_buffer = [0; 256];
        let (fs, relative_path) = self.resolve_path(path, cwd, &mut path_buffer)?;
        { let result = fs.lock().remove(relative_path); result }
    }

    pub fn remove_dir_at(&mut self, cwd: &str, path: &str) -> Result<()> {
        let mut path_buffer = [0; 256];
        let (fs, relative_path) = self.resolve_path(path, cwd, &mut path_buffer)?;
        { let result = fs.lock().remove_dir(relative_path); result }
    }

    pub fn rename_at(&mut self, cwd: &str, old: &str, new: &str) -> Result<()> {
        // Independent buffers keep both endpoint paths valid without allocation.
        let mut old_buffer = [0; 256];
        let mut new_buffer = [0; 256];
        let (old_fs, old_rel) = self.resolve_path(old, cwd, &mut old_buffer)?;
        let (new_fs, new_rel) = self.resolve_path(new, cwd, &mut new_buffer)?;
        if !Arc::ptr_eq(&old_fs, &new_fs) {
            return Err(VfsError::Unsupported);
        }
        { let result = old_fs.lock().rename(old_rel, new_rel); result }
    }

    pub fn truncate_at(&mut self, cwd: &str, path: &str) -> Result<()> {
        let mut path_buffer = [0; 256];
        let (fs, relative_path) = self.resolve_path(path, cwd, &mut path_buffer)?;
        { let result = fs.lock().truncate(relative_path); result }
    }

    pub fn stat_at(&mut self, cwd: &str, path: &str) -> Result<FileStat> {
        let mut path_buffer = [0; 256];
        let (fs, relative_path) = self.resolve_path(path, cwd, &mut path_buffer)?;
        { let result = fs.lock().stat(relative_path); result }
    }

    pub fn normalize_at(&self, cwd: &str, path: &str) -> Result<String> {
        normalize_path(path, cwd)
    }
}

impl Default for VirtualFileSystem {
    fn default() -> Self {
        Self::new()
    }
}

pub fn normalize_path(path: &str, cwd: &str) -> Result<String> {
    let mut buffer = [0u8; 256];
    let normalized = normalize_path_into(path, cwd, &mut buffer)?;
    Ok(String::from(normalized))
}

pub fn normalize_path_into<'a>(path: &str, cwd: &str, out: &'a mut [u8; 256]) -> Result<&'a str> {
    let mut len = 1;
    out[0] = b'/';

    if path.is_empty() {
        append_path_components(cwd, out, &mut len)?;
    } else if !path.starts_with('/') {
        append_path_components(cwd, out, &mut len)?;
        append_path_components(path, out, &mut len)?;
    } else {
        append_path_components(path, out, &mut len)?;
    }

    core::str::from_utf8(&out[..len]).map_err(|_| VfsError::InvalidPath)
}

fn append_path_components(path: &str, out: &mut [u8; 256], len: &mut usize) -> Result<()> {
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => pop_path_component(out, len),
            _ => push_path_component(part, out, len)?,
        }
    }

    Ok(())
}

fn push_path_component(part: &str, out: &mut [u8; 256], len: &mut usize) -> Result<()> {
    let part_bytes = part.as_bytes();
    let needs_slash = *len > 1;
    let required = part_bytes.len() + if needs_slash { 1 } else { 0 };

    if *len + required > out.len() {
        return Err(VfsError::InvalidPath);
    }

    if needs_slash {
        out[*len] = b'/';
        *len += 1;
    }

    out[*len..*len + part_bytes.len()].copy_from_slice(part_bytes);
    *len += part_bytes.len();
    Ok(())
}

fn pop_path_component(out: &[u8; 256], len: &mut usize) {
    if *len <= 1 {
        *len = 1;
        return;
    }

    let mut idx = *len - 1;
    while idx > 0 && out[idx] != b'/' {
        idx -= 1;
    }

    *len = if idx == 0 { 1 } else { idx };
}

// Lock order: registry -> backend. Direct root/capability operations take only
// the backend lock and never re-enter the registry. Process-table lock ordering
// must be resolved before any AP enters these shared syscall paths.
static VFS_INSTANCE: IrqSafeSpinLock<Option<VirtualFileSystem>> = IrqSafeSpinLock::new(None);
pub struct VfsGuard(crate::sync::IrqSafeSpinLockGuard<'static, Option<VirtualFileSystem>>);
impl Deref for VfsGuard {
    type Target = VirtualFileSystem;
    fn deref(&self) -> &Self::Target { self.0.as_ref().expect("initialized VFS") }
}
impl DerefMut for VfsGuard {
    fn deref_mut(&mut self) -> &mut Self::Target { self.0.as_mut().expect("initialized VFS") }
}

static ROOT_MEMFS: OnceCell<Arc<IrqSafeSpinLock<MemFs>>> = OnceCell::new();

fn serial_log(msg: &str) {
    crate::serial::serial_write(msg);
}

/// Create every intermediate directory for `path` (all components except the
/// last) in the root MemFS, idempotently. MemFS `mkdir` is not recursive and
/// rejects an existing or base directory with `AlreadyExists`, so each level is
/// created top-down and errors are ignored. The archive ships files in
/// path-sorted order, but this routine does not rely on that.
fn ensure_parent_dirs(memfs: &mut MemFs, path: &str) {
    let trimmed = path.trim_matches('/');
    let mut parts = trimmed.split('/').peekable();
    let mut prefix = String::new();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            break; // the last component is the file itself, not a directory
        }
        prefix.push('/');
        prefix.push_str(part);
        let _ = memfs.mkdir(&prefix);
    }
}

pub fn init() -> Result<()> {
    serial_log("[VFS] init START\r\n");
    {
        let mut registry = VFS_INSTANCE.lock();
        if registry.is_some() { return Err(VfsError::AlreadyExists); }
        let mut vfs = VirtualFileSystem::new();
        let root = Arc::new(IrqSafeSpinLock::new(MemFs::empty()));
        let mut memfs = root.lock();

        // Populate /app and /assets entirely from the Limine-loaded initrd
        // archive (kernel/src/initrd.rs). The kernel embeds no application
        // binaries or desktop assets; every path comes from the archive. Asset
        // files are mounted zero-copy read-only (they reference module memory,
        // which Limine never reclaims); everything else is copied into a
        // writable MemFS node.
        match crate::initrd::get_initrd() {
            Some(initrd) => {
                for entry in initrd.entries() {
                    ensure_parent_dirs(&mut memfs, &entry.name);
                    if entry.name.starts_with("/assets/") {
                        memfs.add_static_file(&entry.name, entry.data);
                    } else {
                        memfs.add_file(&entry.name, entry.data.to_vec());
                    }
                }
            }
            None => serial_log("[VFS] WARN: no initrd archive; /app and /assets are empty\r\n"),
        }

        // Kernel-seeded writable nodes that are not shipped in the archive.
        let _ = memfs.mkdir("/persist");

        drop(memfs);
        vfs.mount("/", root.clone())?;
        ROOT_MEMFS.set(root).map_err(|_| VfsError::AlreadyExists)?;
        serial_log("[MEMFS] mounted as /\r\n");

        *registry = Some(vfs);
    }

    serial_log("[VFS] init OK\r\n");
    Ok(())
}

pub fn get_vfs() -> Option<VfsGuard> {
    let guard = VFS_INSTANCE.lock();
    if guard.is_some() { Some(VfsGuard(guard)) } else { None }
}

pub fn static_file(path: &str) -> Option<&'static [u8]> {
    ROOT_MEMFS.get()?.lock().static_file(path)
}

pub fn register_device_node(path: &str) {
    if let Some(root) = ROOT_MEMFS.get() { root.lock().add_device(path); }
}

pub fn root_memfs_stats() -> crate::fs::memfs::MemFsStats {
    ROOT_MEMFS.get().expect("root filesystem initialized").lock().stats()
}

/// Boot smoke test (feature `boot-smoke-tests`) exercising the M4 structural
/// FS syscalls at the VFS layer: mkdir, create, rename (file + directory
/// subtree re-home), unlink, the error paths (AlreadyExists / NotFound /
/// IsADirectory) and the read-only guard on `MemData::Static` asset entries.
/// Returns true only if every step behaves as expected.
#[cfg(feature = "boot-smoke-tests")]
pub fn run_fs_mutation_smoke() -> bool {
    fn expect(cond: bool, msg: &str) -> bool {
        if !cond {
            serial_log("[FS-SMOKE] FAIL: ");
            serial_log(msg);
            serial_log("\r\n");
        }
        cond
    }

    let mut vfs = match get_vfs() {
        Some(vfs) => vfs,
        None => return false,
    };
    let root = "/";
    let mut ok = true;

    // Happy path: mkdir under the writable /tmp tree.
    ok &= expect(vfs.mkdir_at(root, "/tmp/fs_smoke").is_ok(), "mkdir /tmp/fs_smoke");
    ok &= expect(
        vfs.mkdir_at(root, "/tmp/fs_smoke") == Err(VfsError::AlreadyExists),
        "mkdir duplicate -> AlreadyExists",
    );

    // Create a file, rename it, verify the old name is gone and new present.
    ok &= expect(vfs.create_at(root, "/tmp/fs_smoke/a.txt").is_ok(), "create a.txt");
    ok &= expect(
        vfs.rename_at(root, "/tmp/fs_smoke/a.txt", "/tmp/fs_smoke/b.txt").is_ok(),
        "rename a.txt -> b.txt",
    );
    ok &= expect(
        vfs.stat_at(root, "/tmp/fs_smoke/a.txt") == Err(VfsError::NotFound),
        "old name gone",
    );
    ok &= expect(vfs.stat_at(root, "/tmp/fs_smoke/b.txt").is_ok(), "new name present");

    // Rename the directory; its descendant file must be re-homed with it.
    ok &= expect(
        vfs.rename_at(root, "/tmp/fs_smoke", "/tmp/fs_smoke2").is_ok(),
        "rename dir -> fs_smoke2",
    );
    ok &= expect(
        vfs.stat_at(root, "/tmp/fs_smoke2/b.txt").is_ok(),
        "descendant re-homed",
    );

    // Unlink the file; a directory cannot be unlinked.
    ok &= expect(vfs.remove_at(root, "/tmp/fs_smoke2/b.txt").is_ok(), "unlink b.txt");
    ok &= expect(
        vfs.stat_at(root, "/tmp/fs_smoke2/b.txt") == Err(VfsError::NotFound),
        "b.txt gone after unlink",
    );
    ok &= expect(
        vfs.remove_at(root, "/tmp/fs_smoke2") == Err(VfsError::IsADirectory),
        "unlink dir -> IsADirectory",
    );

    // Read-only guard: a Static asset entry must reject remove and rename.
    const ASSET: &str = "/assets/fonts/DejaVuSans.ttf";
    ok &= expect(
        vfs.remove_at(root, ASSET) == Err(VfsError::PermissionDenied),
        "remove static asset -> PermissionDenied",
    );
    ok &= expect(
        vfs.rename_at(root, ASSET, "/tmp/stolen.ttf") == Err(VfsError::PermissionDenied),
        "rename static asset -> PermissionDenied",
    );

    ok &= expect(run_backend_ownership_smoke().is_ok(), "backend ownership/close lifetime");
    if ok { serial_log("[VFS-OWNERSHIP] namespace drop, unlink, pinned I/O and final close OK\r\n"); }

    ok
}

/// Open descriptions must keep their backend alive after namespace destruction.
#[cfg(feature = "boot-smoke-tests")]
fn run_backend_ownership_smoke() -> Result<()> {
    fn require_shared<T: Send + Sync>() {}
    require_shared::<super::file_cap::FileCapability>();
    require_shared::<VirtualFileSystem>();
    let root = Arc::new(IrqSafeSpinLock::new(MemFs::empty()));
    root.lock().add_file("/tmp/owned", alloc::vec![1, 2, 3, 4]);
    let weak = Arc::downgrade(&root);
    let mut namespace = VirtualFileSystem::new();
    namespace.mount("/", root.clone())?;
    let cap = namespace.open_capability("/", "/tmp/owned", OpenFlags::READ_WRITE)?;
    let fd = namespace.open_at("/", "/tmp/owned", OpenFlags::READ)?;
    let mut file = namespace.open_files.remove(&fd).ok_or(VfsError::IoError)?;
    drop(namespace);
    drop(root);
    cap.remove()?;
    if cap.write_at(0, b"live")? != 4 { return Err(VfsError::IoError); }
    let mut bytes = [0; 4];
    if file.read(&mut bytes)? != 4 || &bytes != b"live" { return Err(VfsError::IoError); }
    drop(cap);
    if weak.upgrade().ok_or(VfsError::IoError)?.lock().stats().open_handles != 1 {
        return Err(VfsError::IoError);
    }
    drop(file);
    if weak.upgrade().is_some() { return Err(VfsError::IoError); }
    Ok(())
}
