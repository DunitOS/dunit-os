//! Inode/open-description authority. A name is used only when creating the
//! anchor; subsequent operations resolve through that same backend description.
use super::vfs::{Backend, DirEntry, FileHandle, FileStat, OpenFlags, Result, VfsError};
use alloc::{sync::Arc, vec::Vec};

pub struct FileCapability {
    fs: Backend,
    handle: FileHandle,
}
impl FileCapability {
    pub(super) fn new(fs: Backend, handle: FileHandle) -> Self {
        Self { fs, handle }
    }
    pub fn relation(&self, destination: &Self) -> Result<dunit_file_cap_abi::Relation> {
        if !Arc::ptr_eq(&self.fs, &destination.fs) {
            return Ok(dunit_file_cap_abi::Relation::DifferentBackend);
        }
        self.fs.lock().cap_relation(self.handle, destination.handle)
    }
    pub fn sync(&self) -> Result<()> {
        self.fs.lock().fsync(self.handle)
    }
    pub fn stat(&self) -> Result<FileStat> {
        self.fs.lock().stat_handle(self.handle)
    }
    pub fn read_at(&self, offset: u64, bytes: &mut [u8]) -> Result<usize> {
        let offset = i64::try_from(offset).map_err(|_| VfsError::InvalidPath)?;
        let mut fs = self.fs.lock();
        if fs.stat_handle(self.handle)?.file_type != super::vfs::FileType::File {
            return Err(VfsError::IsADirectory);
        }
        fs.seek(self.handle, offset, 0)?;
        fs.read(self.handle, bytes)
    }
    pub fn child(&self, path: &str, flags: OpenFlags) -> Result<Arc<Self>> {
        if !relative(path) {
            return Err(VfsError::InvalidPath);
        }
        let handle = self.fs.lock().cap_child(self.handle, path, flags)?;
        Ok(Arc::new(Self::new(self.fs.clone(), handle)))
    }
    pub fn readdir(&self) -> Result<Vec<DirEntry>> {
        self.fs.lock().cap_readdir(self.handle)
    }
    pub fn write_at(&self, offset: u64, bytes: &[u8]) -> Result<usize> {
        let offset = i64::try_from(offset).map_err(|_| VfsError::InvalidPath)?;
        let mut fs = self.fs.lock();
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
        if !relative(leaf) || leaf.contains('/') {
            return Err(VfsError::InvalidPath);
        }
        let handle = self
            .fs
            .lock()
            .cap_create_child(self.handle, leaf, kind, flags)?;
        Ok(Arc::new(Self::new(self.fs.clone(), handle)))
    }
    pub fn remove(&self) -> Result<()> {
        self.fs.lock().cap_remove(self.handle)
    }
    pub fn move_to(&self, destination: &Self, leaf: &str) -> Result<()> {
        if !relative(leaf) || leaf.contains('/') {
            return Err(VfsError::InvalidPath);
        }
        if !Arc::ptr_eq(&self.fs, &destination.fs) {
            return Err(VfsError::Unsupported);
        }
        self.fs
            .lock()
            .cap_move_to(self.handle, destination.handle, leaf)
    }
}
impl Drop for FileCapability {
    fn drop(&mut self) {
        // The owned backend remains alive until the last description closes.
        let _ = self.fs.lock().close(self.handle);
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
