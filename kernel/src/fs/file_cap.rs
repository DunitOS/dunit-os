//! Inode/open-description authority. A name is used only when creating the
//! anchor; subsequent operations resolve through that same backend description.
use super::vfs::{DirEntry, FileHandle, FileStat, FileSystem, OpenFlags, Result, VfsError};
use alloc::{sync::Arc, vec::Vec};

pub struct FileCapability {
    fs: *mut dyn FileSystem,
    handle: FileHandle,
}
impl FileCapability {
    pub(super) fn new(fs: *mut dyn FileSystem, handle: FileHandle) -> Self {
        Self { fs, handle }
    }
    pub fn relation(&self, destination: &Self) -> Result<dunit_file_cap_abi::Relation> {
        let _irq = crate::sync::InterruptGuard::new();
        if !core::ptr::eq(self.fs, destination.fs) {
            return Ok(dunit_file_cap_abi::Relation::DifferentBackend);
        }
        unsafe { (&mut *self.fs).cap_relation(self.handle, destination.handle) }
    }
    pub fn sync(&self) -> Result<()> {
        let _irq = crate::sync::InterruptGuard::new();
        unsafe { (&mut *self.fs).fsync(self.handle) }
    }
    pub fn stat(&self) -> Result<FileStat> {
        let _irq = crate::sync::InterruptGuard::new();
        unsafe { (&mut *self.fs).stat_handle(self.handle) }
    }
    pub fn read_at(&self, offset: u64, bytes: &mut [u8]) -> Result<usize> {
        let _irq = crate::sync::InterruptGuard::new();
        let offset = i64::try_from(offset).map_err(|_| VfsError::InvalidPath)?;
        let fs = unsafe { &mut *self.fs };
        if fs.stat_handle(self.handle)?.file_type != super::vfs::FileType::File {
            return Err(VfsError::IsADirectory);
        }
        fs.seek(self.handle, offset, 0)?;
        fs.read(self.handle, bytes)
    }
    pub fn child(&self, path: &str, flags: OpenFlags) -> Result<Arc<Self>> {
        let _irq = crate::sync::InterruptGuard::new();
        if !relative(path) {
            return Err(VfsError::InvalidPath);
        }
        let handle = unsafe { (&mut *self.fs).cap_child(self.handle, path, flags)? };
        Ok(Arc::new(Self::new(self.fs, handle)))
    }
    pub fn readdir(&self) -> Result<Vec<DirEntry>> {
        let _irq = crate::sync::InterruptGuard::new();
        unsafe { (&mut *self.fs).cap_readdir(self.handle) }
    }
    pub fn write_at(&self, offset: u64, bytes: &[u8]) -> Result<usize> {
        let _irq = crate::sync::InterruptGuard::new();
        let offset = i64::try_from(offset).map_err(|_| VfsError::InvalidPath)?;
        let fs = unsafe { &mut *self.fs };
        if fs.stat_handle(self.handle)?.file_type != super::vfs::FileType::File {
            return Err(VfsError::IsADirectory);
        }
        fs.seek(self.handle, offset, 0)?;
        fs.write(self.handle, bytes)
    }
    pub fn create_child(
        &self,
        leaf: &str,
        kind: super::vfs::FileType,
        flags: OpenFlags,
    ) -> Result<Arc<Self>> {
        let _irq = crate::sync::InterruptGuard::new();
        if !relative(leaf) || leaf.contains('/') {
            return Err(VfsError::InvalidPath);
        }
        let handle = unsafe { (&mut *self.fs).cap_create_child(self.handle, leaf, kind, flags)? };
        Ok(Arc::new(Self::new(self.fs, handle)))
    }
    pub fn remove(&self) -> Result<()> {
        let _irq = crate::sync::InterruptGuard::new();
        unsafe { (&mut *self.fs).cap_remove(self.handle) }
    }
    pub fn move_to(&self, destination: &Self, leaf: &str) -> Result<()> {
        let _irq = crate::sync::InterruptGuard::new();
        if !relative(leaf) || leaf.contains('/') {
            return Err(VfsError::InvalidPath);
        }
        if !core::ptr::eq(self.fs, destination.fs) {
            return Err(VfsError::Unsupported);
        }
        unsafe { (&mut *self.fs).cap_move_to(self.handle, destination.handle, leaf) }
    }
}
impl Drop for FileCapability {
    fn drop(&mut self) {
        // The UP kernel serializes filesystem access with IRQ masking. The
        // mount-owned backend outlives every process/capability description.
        let _irq = crate::sync::InterruptGuard::new();
        let _ = unsafe { (&mut *self.fs).close(self.handle) };
    }
}
pub(super) fn relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 255
        && !path.as_bytes().contains(&0)
        && !path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
}
