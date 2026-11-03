use super::vfs::{
    DirEntry, FileHandle, FileStat, FileSystem, FileType, OpenFlags, Result, VfsError,
};
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

const BASE_DIRS: [&str; 8] = [
    "kernel", "proc", "app", "assets", "dev", "cfg", "usr", "tmp",
];

/// Upper bound on a single RAM-backed (Owned) file. MemFS data lives in the
/// kernel heap, so without a cap a userspace process that keeps appending can
/// exhaust the heap and abort the kernel. 16 MiB is far above any legitimate
/// temp/config file on this OS (persistent data goes to DunitFS, not here); a
/// write past it returns EFBIG instead of taking the system down.
const MAX_MEMFS_FILE_SIZE: usize = 16 * 1024 * 1024;

struct MemNode {
    id: usize,
    linked: bool,
    path: String,
    file_type: FileType,
    data: MemData,
}

enum MemData {
    Owned(Vec<u8>),
    Static(&'static [u8]),
}

impl MemData {
    fn as_slice(&self) -> &[u8] {
        match self {
            Self::Owned(data) => data.as_slice(),
            Self::Static(data) => data,
        }
    }

    fn len(&self) -> usize {
        self.as_slice().len()
    }

    fn clear(&mut self) -> Result<()> {
        match self {
            Self::Owned(data) => {
                data.clear();
                Ok(())
            }
            Self::Static(_) => Err(VfsError::PermissionDenied),
        }
    }

    fn resize(&mut self, len: usize) -> Result<()> {
        match self {
            Self::Owned(data) => {
                data.resize(len, 0);
                Ok(())
            }
            Self::Static(_) => Err(VfsError::PermissionDenied),
        }
    }

    fn write_at(&mut self, offset: usize, buf: &[u8]) -> Result<()> {
        match self {
            Self::Owned(data) => {
                data[offset..offset + buf.len()].copy_from_slice(buf);
                Ok(())
            }
            Self::Static(_) => Err(VfsError::PermissionDenied),
        }
    }
}

struct OpenMemHandle {
    node_id: usize,
    offset: usize,
    flags: OpenFlags,
}

pub struct MemFs {
    nodes: Vec<MemNode>,
    next_handle: AtomicUsize,
    open_handles: Vec<(FileHandle, OpenMemHandle)>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct MemFsStats {
    pub files: u64,
    pub directories: u64,
    pub bytes: u64,
    pub open_handles: u64,
}

impl MemFs {
    pub const fn empty() -> Self {
        Self {
            nodes: Vec::new(),
            next_handle: AtomicUsize::new(1),
            open_handles: Vec::new(),
        }
    }

    pub fn new() -> Self {
        Self::empty()
    }

    pub fn with_base_tree() -> Self {
        Self::new()
    }

    pub fn add_file(&mut self, name: &str, data: Vec<u8>) {
        let _ = self.create(name);
        if let Some(idx) = self.node_index(name) {
            self.nodes[idx].data = MemData::Owned(data);
        }
    }

    pub fn add_static_file(&mut self, name: &str, data: &'static [u8]) {
        let _ = self.create(name);
        if let Some(idx) = self.node_index(name) {
            self.nodes[idx].data = MemData::Static(data);
        }
    }

    pub fn add_device(&mut self, name: &str) {
        let clean = Self::clean(name);
        if clean.is_empty() || !self.parent_exists(clean) {
            return;
        }

        if let Some(idx) = self.node_index(clean) {
            self.nodes[idx].file_type = FileType::Device;
            self.nodes[idx].data = MemData::Owned(Vec::new());
            return;
        }

        self.nodes.push(MemNode {
            id: self.next_handle.fetch_add(1, Ordering::SeqCst),
            linked: true,
            path: String::from(clean),
            file_type: FileType::Device,
            data: MemData::Owned(Vec::new()),
        });
    }

    pub fn static_file(&self, name: &str) -> Option<&'static [u8]> {
        let idx = self.node_index(name)?;
        match self.nodes[idx].data {
            MemData::Static(data) => Some(data),
            MemData::Owned(_) => None,
        }
    }

    pub fn stats(&self) -> MemFsStats {
        let mut stats = MemFsStats {
            directories: BASE_DIRS.len() as u64,
            ..MemFsStats::default()
        };
        for node in self.nodes.iter().filter(|node| node.linked) {
            match node.file_type {
                FileType::File => {
                    stats.files += 1;
                    stats.bytes += node.data.len() as u64;
                }
                FileType::Directory => stats.directories += 1,
                FileType::Device => {}
            }
        }
        stats.open_handles = self.open_handles.len() as u64;
        stats
    }

    fn clean(path: &str) -> &str {
        path.trim_matches('/')
    }

    fn is_base_dir(path: &str) -> bool {
        let clean = Self::clean(path);
        BASE_DIRS.iter().any(|dir| clean == *dir)
    }

    fn node_index(&self, path: &str) -> Option<usize> {
        let clean = Self::clean(path);
        self.nodes
            .iter()
            .position(|node| node.linked && node.path == clean)
    }

    fn node_type(&self, path: &str) -> Option<FileType> {
        let clean = Self::clean(path);
        if clean.is_empty() || Self::is_base_dir(clean) {
            return Some(FileType::Directory);
        }
        self.node_index(clean).map(|idx| self.nodes[idx].file_type)
    }

    fn parent_path(path: &str) -> &str {
        match Self::clean(path).rfind('/') {
            Some(idx) => &Self::clean(path)[..idx],
            None => "",
        }
    }

    fn basename(path: &str) -> &str {
        match Self::clean(path).rfind('/') {
            Some(idx) => &Self::clean(path)[idx + 1..],
            None => Self::clean(path),
        }
    }

    fn parent_exists(&self, path: &str) -> bool {
        self.node_type(Self::parent_path(path)) == Some(FileType::Directory)
    }

    fn handle_index(&self, handle: FileHandle) -> Option<usize> {
        self.open_handles.iter().position(|(id, _)| *id == handle)
    }

    fn capability_path(&self, handle: FileHandle) -> Result<String> {
        let h = &self.open_handles[self.handle_index(handle).ok_or(VfsError::InvalidDescriptor)?].1;
        if let Some(i) = usize::MAX.checked_sub(h.node_id).filter(|i| *i < BASE_DIRS.len()) {
            return Ok(String::from(BASE_DIRS[i]));
        }
        self.nodes.iter().find(|n| n.id == h.node_id && n.linked)
            .map(|n| n.path.clone()).ok_or(VfsError::NotFound)
    }

    fn is_direct_child(parent: &str, child: &str) -> bool {
        let parent = Self::clean(parent);
        let child = Self::clean(child);

        if parent.is_empty() {
            !child.is_empty() && !child.contains('/')
        } else {
            child.starts_with(parent)
                && child.len() > parent.len()
                && child.as_bytes()[parent.len()] == b'/'
                && !child[parent.len() + 1..].contains('/')
        }
    }
}

impl FileSystem for MemFs {
    fn cap_relation(&mut self,source:FileHandle,destination:FileHandle)->Result<dunit_file_cap_abi::Relation>{
        let source=self.capability_path(source)?;let destination=self.capability_path(destination)?;
        Ok(dunit_file_cap_abi::Relation::paths(&source,&destination))
    }
    fn cap_open(&mut self, path: &str, flags: OpenFlags) -> Result<FileHandle> {
        let clean = Self::clean(path);
        // The virtual root spans mount points, not one backend inode.
        if clean.is_empty() { return Err(VfsError::Unsupported); }
        let node_id = if let Some(i) = BASE_DIRS.iter().position(|p| *p == clean) {
            usize::MAX - i
        } else {
            self.nodes[self.node_index(clean).ok_or(VfsError::NotFound)?].id
        };
        let handle = self.next_handle.fetch_add(1, Ordering::SeqCst);
        self.open_handles.push((handle, OpenMemHandle { node_id, offset: 0, flags }));
        Ok(handle)
    }
    fn cap_child(&mut self, parent: FileHandle, path: &str, flags: OpenFlags) -> Result<FileHandle> {
        if !super::file_cap::relative(path) { return Err(VfsError::InvalidPath); }
        let base = self.capability_path(parent)?;
        if self.stat_handle(parent)?.file_type != FileType::Directory { return Err(VfsError::NotADirectory); }
        let mut child=base; child.push('/'); child.push_str(path);
        self.cap_open(&child, flags)
    }
    fn cap_readdir(&mut self, handle: FileHandle) -> Result<Vec<DirEntry>> {
        let path = self.capability_path(handle)?;
        if self.stat_handle(handle)?.file_type != FileType::Directory { return Err(VfsError::NotADirectory); }
        if self.nodes.iter().filter(|n|n.linked&&Self::is_direct_child(&path,&n.path))
            .any(|n|Self::basename(&n.path).len()>64) {return Err(VfsError::FileTooBig);}
        self.readdir(&path)
    }
    fn cap_remove(&mut self, handle: FileHandle) -> Result<()> {
        let index = self.handle_index(handle).ok_or(VfsError::InvalidDescriptor)?;
        if !self.open_handles[index].1.flags.can_write() { return Err(VfsError::PermissionDenied); }
        let path = self.capability_path(handle)?;
        match self.stat_handle(handle)?.file_type {
            FileType::File => self.remove(&path),
            FileType::Directory => self.remove_dir(&path),
            FileType::Device => Err(VfsError::Unsupported),
        }
    }
    fn cap_move_to(&mut self, source: FileHandle, destination: FileHandle, leaf: &str) -> Result<()> {
        if !super::file_cap::relative(leaf) || leaf.contains('/') { return Err(VfsError::InvalidPath); }
        for h in [source,destination] {
            let i=self.handle_index(h).ok_or(VfsError::InvalidDescriptor)?;
            if !self.open_handles[i].1.flags.can_write() { return Err(VfsError::PermissionDenied); }
        }
        let old=self.capability_path(source)?;
        let mut new=self.capability_path(destination)?;
        if self.stat_handle(destination)?.file_type != FileType::Directory { return Err(VfsError::NotADirectory); }
        new.push('/');new.push_str(leaf);
        self.rename(&old,&new)
    }
    fn cap_create_child(&mut self,parent:FileHandle,leaf:&str,kind:FileType,flags:OpenFlags)->Result<FileHandle> {
        if !super::file_cap::relative(leaf)||leaf.contains('/'){return Err(VfsError::InvalidPath);}
        let i=self.handle_index(parent).ok_or(VfsError::InvalidDescriptor)?;
        if !self.open_handles[i].1.flags.can_write(){return Err(VfsError::PermissionDenied);}
        if self.stat_handle(parent)?.file_type!=FileType::Directory{return Err(VfsError::NotADirectory);}
        let mut path=self.capability_path(parent)?;path.push('/');path.push_str(leaf);
        match kind {FileType::File=>self.create(&path)?,FileType::Directory=>self.mkdir(&path)?,FileType::Device=>return Err(VfsError::Unsupported)}
        self.cap_open(&path,flags)
    }
    fn open(&mut self, path: &str, flags: OpenFlags) -> Result<FileHandle> {
        if !flags.is_valid() {
            return Err(VfsError::InvalidPath);
        }

        let exists = self.node_type(path).is_some();
        // O_EXCL: exclusive create must fail if the path already exists.
        if flags.create() && flags.is_exclusive() && exists {
            return Err(VfsError::AlreadyExists);
        }
        if !exists && flags.create() {
            self.create(path)?;
        }

        match self.node_type(path).ok_or(VfsError::NotFound)? {
            FileType::Directory => Err(VfsError::IsADirectory),
            FileType::File => {
                let nidx = self.node_index(path).ok_or(VfsError::NotFound)?;
                if flags.trunc() {
                    if !flags.can_write() {
                        return Err(VfsError::PermissionDenied);
                    }
                    self.nodes[nidx].data.clear()?;
                }

                let handle = self.next_handle.fetch_add(1, Ordering::SeqCst);
                let offset = if flags.append() {
                    self.nodes[nidx].data.len()
                } else {
                    0
                };
                self.open_handles.push((
                    handle,
                    OpenMemHandle {
                        node_id: self.nodes[nidx].id,
                        offset,
                        flags,
                    },
                ));
                Ok(handle)
            }
            FileType::Device => Err(VfsError::Unsupported),
        }
    }

    fn read(&mut self, handle: FileHandle, buf: &mut [u8]) -> Result<usize> {
        let hidx = self
            .handle_index(handle)
            .ok_or(VfsError::InvalidDescriptor)?;
        if !self.open_handles[hidx].1.flags.can_read() {
            return Err(VfsError::PermissionDenied);
        }

        let node_id = self.open_handles[hidx].1.node_id;
        let offset = self.open_handles[hidx].1.offset;
        let nidx = self
            .nodes
            .iter()
            .position(|node| node.id == node_id)
            .ok_or(VfsError::InvalidDescriptor)?;

        if self.nodes[nidx].file_type != FileType::File {
            return Err(VfsError::IsADirectory);
        }

        let data = self.nodes[nidx].data.as_slice();
        let bytes_read = if offset >= data.len() {
            0
        } else {
            let len = buf.len().min(data.len() - offset);
            buf[..len].copy_from_slice(&data[offset..offset + len]);
            len
        };

        self.open_handles[hidx].1.offset += bytes_read;
        Ok(bytes_read)
    }

    fn write(&mut self, handle: FileHandle, buf: &[u8]) -> Result<usize> {
        let hidx = self
            .handle_index(handle)
            .ok_or(VfsError::InvalidDescriptor)?;
        if !self.open_handles[hidx].1.flags.can_write() {
            return Err(VfsError::PermissionDenied);
        }

        let node_id = self.open_handles[hidx].1.node_id;
        let nidx = self
            .nodes
            .iter()
            .position(|node| node.id == node_id)
            .ok_or(VfsError::InvalidDescriptor)?;

        if self.nodes[nidx].file_type != FileType::File {
            return Err(VfsError::IsADirectory);
        }

        let offset = if self.open_handles[hidx].1.flags.append() {
            self.nodes[nidx].data.len()
        } else {
            self.open_handles[hidx].1.offset
        };
        let end = offset + buf.len();
        // Cap a single RAM file so a runaway writer cannot grow it until the
        // kernel heap is exhausted (an unbounded write used to OOM-abort the
        // whole kernel). Honest error (EFBIG) instead of taking the system down.
        if end > MAX_MEMFS_FILE_SIZE {
            return Err(VfsError::FileTooBig);
        }
        if end > self.nodes[nidx].data.len() {
            self.nodes[nidx].data.resize(end)?;
        }
        self.nodes[nidx].data.write_at(offset, buf)?;
        self.open_handles[hidx].1.offset = end;
        Ok(buf.len())
    }

    fn close(&mut self, handle: FileHandle) -> Result<()> {
        let idx = self
            .handle_index(handle)
            .ok_or(VfsError::InvalidDescriptor)?;
        let node_id = self.open_handles.remove(idx).1.node_id;
        if !self
            .open_handles
            .iter()
            .any(|(_, handle)| handle.node_id == node_id)
        {
            self.nodes.retain(|node| node.id != node_id || node.linked);
        }
        Ok(())
    }

    fn seek(&mut self, handle: FileHandle, offset: i64, whence: u32) -> Result<u64> {
        let hidx = self
            .handle_index(handle)
            .ok_or(VfsError::InvalidDescriptor)?;
        let node_id = self.open_handles[hidx].1.node_id;
        let nidx = self
            .nodes
            .iter()
            .position(|node| node.id == node_id)
            .ok_or(VfsError::InvalidDescriptor)?;
        if self.nodes[nidx].file_type != FileType::File {
            return Err(VfsError::IsADirectory);
        }
        let size = self.nodes[nidx].data.len() as i64;
        let base = match whence {
            0 => 0,                                       // SEEK_SET
            1 => self.open_handles[hidx].1.offset as i64, // SEEK_CUR
            2 => size,                                    // SEEK_END
            _ => return Err(VfsError::InvalidPath),
        };
        let new = base.checked_add(offset).ok_or(VfsError::InvalidPath)?;
        if new < 0 {
            return Err(VfsError::InvalidPath);
        }
        self.open_handles[hidx].1.offset = new as usize;
        Ok(new as u64)
    }

    fn stat_handle(&mut self, handle: FileHandle) -> Result<FileStat> {
        let hidx = self
            .handle_index(handle)
            .ok_or(VfsError::InvalidDescriptor)?;
        let node_id = self.open_handles[hidx].1.node_id;
        if usize::MAX.checked_sub(node_id).is_some_and(|i| i < BASE_DIRS.len()) {
            return Ok(FileStat { file_type: FileType::Directory, size: 0 });
        }
        let node = self
            .nodes
            .iter()
            .find(|node| node.id == node_id)
            .ok_or(VfsError::InvalidDescriptor)?;
        Ok(FileStat {
            file_type: node.file_type,
            size: node.data.len(),
        })
    }

    fn ftruncate_handle(&mut self, handle: FileHandle, len: u64) -> Result<()> {
        let hidx = self
            .handle_index(handle)
            .ok_or(VfsError::InvalidDescriptor)?;
        if !self.open_handles[hidx].1.flags.can_write() {
            return Err(VfsError::PermissionDenied);
        }
        let node_id = self.open_handles[hidx].1.node_id;
        let nidx = self
            .nodes
            .iter()
            .position(|node| node.id == node_id)
            .ok_or(VfsError::InvalidDescriptor)?;
        if self.nodes[nidx].file_type != FileType::File {
            return Err(VfsError::IsADirectory);
        }
        let len = len as usize;
        if len > MAX_MEMFS_FILE_SIZE {
            return Err(VfsError::FileTooBig);
        }
        // ftruncate changes only the size (grow with zeros / shrink); the
        // handle's offset is intentionally left where it is (POSIX).
        self.nodes[nidx].data.resize(len)
    }

    fn readdir(&mut self, path: &str) -> Result<Vec<DirEntry>> {
        let clean = Self::clean(path);
        let mut entries = Vec::new();

        if clean.is_empty() {
            for base_dir in BASE_DIRS.iter() {
                entries.push(DirEntry::new(base_dir, FileType::Directory));
            }
            for node in self.nodes.iter().filter(|node| node.linked) {
                if Self::is_direct_child("", &node.path) && !Self::is_base_dir(&node.path) {
                    entries.push(DirEntry::new(Self::basename(&node.path), node.file_type));
                }
            }
            return Ok(entries);
        }

        let kind = self.node_type(clean).ok_or(VfsError::NotFound)?;
        if kind != FileType::Directory {
            return Err(VfsError::NotADirectory);
        }

        for node in self.nodes.iter().filter(|node| node.linked) {
            if Self::is_direct_child(clean, &node.path) {
                entries.push(DirEntry::new(Self::basename(&node.path), node.file_type));
            }
        }

        Ok(entries)
    }

    fn readdir_into(&mut self, path: &str, entries: &mut [DirEntry]) -> Result<usize> {
        let mut count = 0;
        let clean = Self::clean(path);

        if clean.is_empty() {
            for base_dir in BASE_DIRS.iter() {
                if count < entries.len() {
                    entries[count] = DirEntry::new(base_dir, FileType::Directory);
                    count += 1;
                }
            }

            for node in self.nodes.iter().filter(|node| node.linked) {
                if Self::is_direct_child("", &node.path) && !Self::is_base_dir(&node.path) {
                    if count < entries.len() {
                        entries[count] = DirEntry::new(Self::basename(&node.path), node.file_type);
                        count += 1;
                    }
                }
            }
            return Ok(count);
        }

        let kind = self.node_type(clean).ok_or(VfsError::NotFound)?;
        if kind != FileType::Directory {
            return Err(VfsError::NotADirectory);
        }

        for node in self.nodes.iter().filter(|node| node.linked) {
            if Self::is_direct_child(clean, &node.path) {
                if count < entries.len() {
                    entries[count] = DirEntry::new(Self::basename(&node.path), node.file_type);
                    count += 1;
                }
            }
        }

        Ok(count)
    }

    fn create(&mut self, path: &str) -> Result<()> {
        let clean = Self::clean(path);
        if clean.is_empty() {
            return Err(VfsError::InvalidPath);
        }
        if self.node_type(clean).is_some() {
            return Err(VfsError::AlreadyExists);
        }
        if !self.parent_exists(clean) {
            return Err(VfsError::NotFound);
        }

        self.nodes.push(MemNode {
            id: self.next_handle.fetch_add(1, Ordering::SeqCst),
            linked: true,
            path: String::from(clean),
            file_type: FileType::File,
            data: MemData::Owned(Vec::new()),
        });
        Ok(())
    }

    fn mkdir(&mut self, path: &str) -> Result<()> {
        let clean = Self::clean(path);
        if clean.is_empty() {
            return Err(VfsError::InvalidPath);
        }
        if self.node_type(clean).is_some() {
            return Err(VfsError::AlreadyExists);
        }
        if !self.parent_exists(clean) {
            return Err(VfsError::NotFound);
        }

        self.nodes.push(MemNode {
            id: self.next_handle.fetch_add(1, Ordering::SeqCst),
            linked: true,
            path: String::from(clean),
            file_type: FileType::Directory,
            data: MemData::Owned(Vec::new()),
        });
        Ok(())
    }

    fn remove(&mut self, path: &str) -> Result<()> {
        let clean = Self::clean(path);
        if clean.is_empty() {
            return Err(VfsError::InvalidPath);
        }
        let idx = self.node_index(clean).ok_or(VfsError::NotFound)?;
        // Only Owned nodes are mutable; asset/static entries are read-only.
        if matches!(self.nodes[idx].data, MemData::Static(_)) {
            return Err(VfsError::PermissionDenied);
        }
        if self.nodes[idx].file_type != FileType::File {
            return Err(VfsError::IsADirectory);
        }
        let node_id = self.nodes[idx].id;
        if self
            .open_handles
            .iter()
            .any(|(_, handle)| handle.node_id == node_id)
        {
            self.nodes[idx].linked = false;
        } else {
            self.nodes.remove(idx);
        }
        Ok(())
    }

    fn remove_dir(&mut self, path: &str) -> Result<()> {
        let clean = Self::clean(path);
        if clean.is_empty() {
            return Err(VfsError::InvalidPath);
        }
        let idx = self.node_index(clean).ok_or(VfsError::NotFound)?;
        if matches!(self.nodes[idx].data, MemData::Static(_)) {
            return Err(VfsError::PermissionDenied);
        }
        if self.nodes[idx].file_type != FileType::Directory {
            return Err(VfsError::NotADirectory);
        }
        // Reject non-empty directories (any node living beneath this path).
        let mut prefix = String::from(clean);
        prefix.push('/');
        if self
            .nodes
            .iter()
            .any(|node| node.linked && node.path.starts_with(prefix.as_str()))
        {
            return Err(VfsError::DirectoryNotEmpty);
        }
        let node_id = self.nodes[idx].id;
        if self.open_handles.iter().any(|(_, h)| h.node_id == node_id) {
            self.nodes[idx].linked = false;
        } else {
            self.nodes.remove(idx);
        }
        Ok(())
    }

    fn rename(&mut self, old: &str, new: &str) -> Result<()> {
        let old_clean = Self::clean(old);
        let new_clean = Self::clean(new);
        if old_clean.is_empty() || new_clean.is_empty() {
            return Err(VfsError::InvalidPath);
        }
        if old_clean == new_clean {
            return Ok(());
        }
        // A directory cannot be moved inside its own subtree.
        let mut old_slash = String::from(old_clean);
        old_slash.push('/');
        if new_clean.starts_with(&old_slash) {
            return Err(VfsError::InvalidPath);
        }

        let idx = self.node_index(old_clean).ok_or(VfsError::NotFound)?;
        // Only Owned nodes are mutable; asset/static entries are read-only.
        if matches!(self.nodes[idx].data, MemData::Static(_)) {
            return Err(VfsError::PermissionDenied);
        }
        if self.node_type(new_clean).is_some() {
            return Err(VfsError::AlreadyExists);
        }
        if !self.parent_exists(new_clean) {
            return Err(VfsError::NotFound);
        }

        let is_dir = self.nodes[idx].file_type == FileType::Directory;
        self.nodes[idx].path = String::from(new_clean);

        if is_dir {
            // Re-home every descendant: "<old>/<rest>" -> "<new>/<rest>".
            for node in self.nodes.iter_mut() {
                if node.linked && node.path.starts_with(&old_slash) {
                    let mut moved = String::from(new_clean);
                    moved.push_str(&node.path[old_clean.len()..]);
                    node.path = moved;
                }
            }
        }
        // Open descriptors refer to stable node identities, so renaming a name
        // never invalidates or retargets an existing open description.
        Ok(())
    }

    fn truncate(&mut self, path: &str) -> Result<()> {
        let clean = Self::clean(path);
        if clean.is_empty() {
            return Err(VfsError::InvalidPath);
        }
        let idx = self.node_index(clean).ok_or(VfsError::NotFound)?;
        if self.nodes[idx].file_type != FileType::File {
            return Err(VfsError::IsADirectory);
        }
        self.nodes[idx].data.clear()?;
        Ok(())
    }

    fn stat(&mut self, path: &str) -> Result<FileStat> {
        let clean = Self::clean(path);
        if clean.is_empty() || Self::is_base_dir(clean) {
            return Ok(FileStat {
                file_type: FileType::Directory,
                size: 0,
            });
        }

        let idx = self.node_index(clean).ok_or(VfsError::NotFound)?;
        Ok(FileStat {
            file_type: self.nodes[idx].file_type,
            size: self.nodes[idx].data.len(),
        })
    }
}

impl Default for MemFs {
    fn default() -> Self {
        Self::new()
    }
}
