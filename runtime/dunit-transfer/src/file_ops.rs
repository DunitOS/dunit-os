//! Capability-scoped, no-overwrite copy. The same job serves synchronous tools
//! and GUI clients; one poll copies at most one 4 KiB chunk or visits one node.
//! Move is a separate broker operation and never a copy/unlink fallback.
use super::*;
use libdunit::file_cap::{self as cap, Relation};
const MAX_DEPTH: usize = 64;
const MAX_ENTRIES: usize = 16_384;
const MAX_BYTES: u64 = 256 * 1024 * 1024;
const COPY_RIGHTS: u32 = libdunit::RIGHT_READ | libdunit::RIGHT_WRITE;
#[derive(Debug, PartialEq, Eq)]
pub struct CopyError {
    pub operation: Error,
    pub cleanup: Option<Error>,
}
enum Task {
    Node {
        source: File,
        destination: OwnedHandle,
        leaf: String,
        depth: usize,
    },
    Directory {
        source: File,
        destination: OwnedHandle,
        entries: alloc::vec::IntoIter<libdunit::DirEntry>,
        depth: usize,
    },
    Bytes {
        source: File,
        destination: OwnedHandle,
        size: u64,
        offset: u64,
    },
}
/// Retains source/destination authority throughout the job. Failure/cancellation
/// removes only nodes this job created, in reverse order, by identity. Unknown
/// concurrent children are never recursively removed.
pub struct CopyJob {
    tasks: Vec<Task>,
    created: Vec<OwnedHandle>,
    entries: usize,
    bytes: u64,
    finished: bool,
    failure: Option<CopyError>,
}
impl CopyJob {
    pub fn new(source: &File, destination: &OwnedHandle, leaf: &str) -> NativeResult<Self> {
        let source = File {
            handle: source.handle.duplicate(libdunit::RIGHT_READ)?,
            path: source.path.clone(),
            kind: source.kind,
        };
        let destination = destination.duplicate(COPY_RIGHTS)?;
        Ok(Self {
            tasks: alloc::vec![Task::Node {
                source,
                destination,
                leaf: String::from(leaf),
                depth: 0
            }],
            created: Vec::new(),
            entries: 0,
            bytes: 0,
            finished: false,
            failure: None,
        })
    }
    fn cleanup(&mut self) -> Option<Error> {
        self.tasks.clear();
        let mut error = None;
        while let Some(handle) = self.created.pop() {
            let result = cap::remove(handle.raw());
            if result < 0 && error.is_none() {
                error = Some(Error(result));
            }
        }
        self.finished = true;
        error
    }
    /// Begin cancellation; GUI callers keep polling so cleanup also yields.
    pub fn request_cancel(&mut self) {
        if !self.finished && self.failure.is_none() {
            self.tasks.clear();
            self.failure = Some(CopyError {
                operation: Error(libdunit::EINTR),
                cleanup: None,
            });
        }
    }
    pub fn cancel(&mut self) -> core::result::Result<(), CopyError> {
        self.request_cancel();
        while !self.poll()? {}
        Ok(())
    }
    /// True only after all nodes have been copied and explicitly synchronized.
    /// Failure/cancellation removes at most one created node per subsequent poll.
    pub fn poll(&mut self) -> core::result::Result<bool, CopyError> {
        if self.finished {
            return Ok(true);
        }
        if self.failure.is_some() {
            if let Some(handle) = self.created.pop() {
                let result = cap::remove(handle.raw());
                let error = self.failure.as_mut().unwrap();
                if result < 0 && error.cleanup.is_none() {
                    error.cleanup = Some(Error(result));
                }
            }
            if self.created.is_empty() {
                self.finished = true;
                return Err(self.failure.take().unwrap());
            }
            return Ok(false);
        }
        match self.step() {
            Ok(()) if self.tasks.is_empty() => {
                self.created.clear();
                self.finished = true;
                Ok(true)
            }
            Ok(()) => Ok(false),
            Err(operation) => {
                self.tasks.clear();
                self.failure = Some(CopyError {
                    operation,
                    cleanup: None,
                });
                self.poll()
            }
        }
    }
    fn step(&mut self) -> NativeResult<()> {
        let Some(task) = self.tasks.pop() else {
            return Ok(());
        };
        match task {
            Task::Node {
                source,
                destination,
                leaf,
                depth,
            } => {
                if depth >= MAX_DEPTH || self.entries >= MAX_ENTRIES {
                    return Err(Error(libdunit::EFBIG));
                }
                let before = source.stat()?;
                let kind = match before.file_type {
                    libdunit::FILE_TYPE_FILE => FileKind::File,
                    libdunit::FILE_TYPE_DIRECTORY => FileKind::Directory,
                    _ => return Err(Error(libdunit::EOPNOTSUPP)),
                };
                if kind != source.kind {
                    return Err(Error(libdunit::EINVAL));
                }
                if kind == FileKind::Directory {
                    let relation =
                        checked(cap::relation(source.handle.raw(), destination.raw()))? as u32;
                    if matches!(
                        Relation::from_raw(relation),
                        Some(Relation::Same | Relation::SourceAncestor)
                    ) {
                        return Err(Error(libdunit::EINVAL));
                    }
                    if Relation::from_raw(relation).is_none() {
                        return Err(Error(libdunit::EIO));
                    }
                }
                if kind == FileKind::File
                    && self.bytes.saturating_add(before.size as u64) > MAX_BYTES
                {
                    return Err(Error(libdunit::EFBIG));
                }
                let handle = OwnedHandle(checked(cap::create_child(
                    destination.raw(),
                    &leaf,
                    kind as u32,
                    COPY_RIGHTS,
                ))? as u32);
                self.created.push(handle);
                let destination = self
                    .created
                    .last()
                    .ok_or(Error(libdunit::EIO))?
                    .duplicate(COPY_RIGHTS)?;
                self.entries += 1;
                match kind {
                    FileKind::Directory => {
                        let entries = source.entries()?.into_iter();
                        self.tasks.push(Task::Directory {
                            source,
                            destination,
                            entries,
                            depth,
                        });
                    }
                    FileKind::File => {
                        self.bytes += before.size as u64;
                        self.tasks.push(Task::Bytes {
                            source,
                            destination,
                            size: before.size as u64,
                            offset: 0,
                        });
                    }
                }
            }
            Task::Directory {
                source,
                destination,
                mut entries,
                depth,
            } => {
                if let Some(entry) = entries.next() {
                    let child = source.child(entry.name())?;
                    let target = destination.duplicate(COPY_RIGHTS)?;
                    self.tasks.push(Task::Directory {
                        source,
                        destination,
                        entries,
                        depth,
                    });
                    self.tasks.push(Task::Node {
                        source: child,
                        destination: target,
                        leaf: String::from(entry.name()),
                        depth: depth + 1,
                    });
                } else {
                    checked(cap::sync(destination.raw()))?;
                }
            }
            Task::Bytes {
                source,
                destination,
                size,
                offset,
            } => {
                if offset == size {
                    if source.stat()?.size as u64 != size {
                        return Err(Error(libdunit::EAGAIN));
                    }
                    checked(cap::sync(destination.raw()))?;
                } else {
                    let mut bytes = [0u8; 4096];
                    let count = (size - offset).min(bytes.len() as u64) as usize;
                    let read = source.read_at(offset, &mut bytes[..count])?;
                    if read == 0 || read > count {
                        return Err(Error(libdunit::EIO));
                    }
                    let mut written = 0;
                    while written < read {
                        let n = checked(cap::write_at(
                            destination.raw(),
                            offset + written as u64,
                            &bytes[written..read],
                        ))?;
                        if n == 0 || n > read - written {
                            return Err(Error(libdunit::EIO));
                        }
                        written += n;
                    }
                    self.tasks.push(Task::Bytes {
                        source,
                        destination,
                        size,
                        offset: offset + read as u64,
                    });
                }
            }
        }
        Ok(())
    }
}
impl Drop for CopyJob {
    fn drop(&mut self) {
        if !self.finished {
            if let Some(error) = self.cleanup() {
                libdunit::println(&alloc::format!(
                    "copy cleanup: {}",
                    libdunit::error_name(error.0)
                ));
            }
        }
    }
}
pub fn copy_into(
    source: &File,
    destination: &OwnedHandle,
    leaf: &str,
) -> core::result::Result<(), CopyError> {
    let mut job = CopyJob::new(source, destination, leaf).map_err(|operation| CopyError {
        operation,
        cleanup: None,
    })?;
    while !job.poll()? {}
    Ok(())
}
