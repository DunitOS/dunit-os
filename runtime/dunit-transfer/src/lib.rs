//! Native transfer client and capability custody. The embedding window owns
//! the IPC inbox: this crate never receives a packet independently.
#![no_std]
extern crate alloc;
use alloc::{string::String, vec::Vec};
pub use gui_protocol_v1::transfer::*;
pub use libdunit::{RIGHT_READ, RIGHT_TRANSFER, RIGHT_WRITE};
mod client;
pub mod file_ops;
pub mod transport;
pub use client::{Client, Event};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error(pub isize);
pub type NativeResult<T> = core::result::Result<T, Error>;
pub(crate) fn checked(n: isize) -> NativeResult<usize> {
    if n < 0 {
        Err(Error(n))
    } else {
        Ok(n as usize)
    }
}
pub(crate) fn invalid<T>(_: T) -> Error {
    Error(libdunit::EINVAL)
}

/// Exactly one locally owned kernel handle. No Clone: duplicate explicitly
/// with attenuated rights, or move the owner into a payload/event.
#[derive(Debug)]
pub struct OwnedHandle(u32);
impl OwnedHandle {
    /// `handle` must be a newly created or kernel-attested received handle,
    /// whose ownership has not been adopted elsewhere. Never use a wire field.
    pub unsafe fn adopt(handle: u32) -> Self {
        Self(handle)
    }
    pub fn raw(&self) -> u32 {
        self.0
    }
    pub fn duplicate(&self, rights: u32) -> NativeResult<Self> {
        Ok(Self(checked(libdunit::handle_dup(self.0, rights))? as u32))
    }
    pub fn rights(&self) -> NativeResult<u32> {
        checked(libdunit::handle_rights(self.0)).map(|n| n as u32)
    }
}
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        libdunit::handle_close(self.0);
    }
}
#[derive(Debug)]
pub struct File {
    pub handle: OwnedHandle,
    pub path: String,
    pub kind: FileKind,
}
impl File {
    /// Read one bounded file through its held authority, never through its
    /// display path. A concurrent size change fails the complete read.
    pub fn read_all(&self, limit: usize) -> NativeResult<Vec<u8>> {
        if self.kind != FileKind::File {
            return Err(Error(libdunit::EISDIR));
        }
        let before = self.stat()?;
        let size = before.size as usize;
        if size > limit {
            return Err(Error(libdunit::EFBIG));
        }
        let mut bytes = alloc::vec![0;size];
        let mut offset = 0;
        while offset < size {
            let end = (offset + 4096).min(size);
            let n = self.read_at(offset as u64, &mut bytes[offset..end])?;
            if n == 0 || n > end - offset {
                return Err(Error(libdunit::EIO));
            }
            offset += n;
        }
        if self.stat()?.size != before.size {
            return Err(Error(libdunit::EAGAIN));
        }
        Ok(bytes)
    }
    pub fn open(path: &str, writable: bool) -> NativeResult<Self> {
        let rights = libdunit::RIGHT_READ
            | libdunit::RIGHT_TRANSFER
            | if writable { libdunit::RIGHT_WRITE } else { 0 };
        let handle = OwnedHandle(checked(libdunit::file_cap::open(path, rights))? as u32);
        let kind = stat_kind(&handle)?;
        let value = Self {
            handle,
            path: String::from(path),
            kind,
        };
        FileList {
            items: alloc::vec![value.item()],
        }
        .validate()
        .map_err(invalid)?;
        Ok(value)
    }
    pub fn item(&self) -> FileItem {
        FileItem {
            path: self.path.clone(),
            kind: self.kind,
            capability: self.handle.raw(),
        }
    }
    pub fn read_at(&self, offset: u64, bytes: &mut [u8]) -> NativeResult<usize> {
        checked(libdunit::file_cap::read_at(
            self.handle.raw(),
            offset,
            bytes,
        ))
    }
    pub fn stat(&self) -> NativeResult<libdunit::FileStat> {
        let mut stat = libdunit::FileStat::default();
        checked(libdunit::file_cap::stat(self.handle.raw(), &mut stat))?;
        Ok(stat)
    }
    pub fn child(&self, path: &str) -> NativeResult<Self> {
        self.child_with_rights(path, false)
    }
    pub fn child_with_rights(&self, path: &str, writable: bool) -> NativeResult<Self> {
        let rights = libdunit::RIGHT_READ
            | (self.handle.rights()? & libdunit::RIGHT_TRANSFER)
            | if writable { libdunit::RIGHT_WRITE } else { 0 };
        let handle = OwnedHandle(checked(libdunit::file_cap::open_child(
            self.handle.raw(),
            path,
            rights,
        ))? as u32);
        let kind = stat_kind(&handle)?;
        Ok(Self {
            handle,
            path: alloc::format!("{}/{}", self.path.trim_end_matches('/'), path),
            kind,
        })
    }
    pub fn entries(&self) -> NativeResult<Vec<libdunit::DirEntry>> {
        let n = checked(libdunit::file_cap::readdir(self.handle.raw(), &mut []))?;
        if n > 1024 {
            return Err(Error(libdunit::ENOBUFS));
        }
        let mut entries = alloc::vec![libdunit::DirEntry::empty();n];
        let actual = checked(libdunit::file_cap::readdir(self.handle.raw(), &mut entries))?;
        if actual > n {
            return Err(Error(libdunit::EAGAIN));
        }
        entries.truncate(actual);
        Ok(entries)
    }
}
pub(crate) fn stat_kind(handle: &OwnedHandle) -> NativeResult<FileKind> {
    let mut stat = libdunit::FileStat::default();
    checked(libdunit::file_cap::stat(handle.raw(), &mut stat))?;
    match stat.file_type {
        1 => Ok(FileKind::File),
        2 => Ok(FileKind::Directory),
        _ => Err(Error(libdunit::EOPNOTSUPP)),
    }
}
#[derive(Debug)]
pub struct Payload {
    pub bundle: Bundle,
    pub files: Vec<File>,
}
impl Payload {
    /// Every native file source supplies explicit URI/text alternatives so
    /// text targets never need filesystem capabilities just to insert paths.
    pub fn from_files(files: Vec<File>) -> NativeResult<Self> {
        let items = files.iter().map(File::item).collect();
        let paths = PathList {
            paths: files.iter().map(|f| f.path.clone()).collect(),
        };
        let plain = paths.paths.join("\n");
        let bundle = Bundle {
            representations: alloc::vec![
                Representation {
                    format: Format::Files,
                    data: FileList { items }.encode().map_err(invalid)?
                },
                Representation {
                    format: Format::Custom(String::from(URI_LIST_MIME)),
                    data: paths.encode().map_err(invalid)?
                },
                Representation {
                    format: Format::Text,
                    data: plain.into_bytes()
                },
            ],
        };
        Self::new(bundle, files)
    }
    pub fn text(text: &str) -> NativeResult<Self> {
        Self::new(
            Bundle {
                representations: alloc::vec![Representation {
                    format: Format::Text,
                    data: text.as_bytes().to_vec()
                }],
            },
            Vec::new(),
        )
    }
    pub fn new(bundle: Bundle, files: Vec<File>) -> NativeResult<Self> {
        bundle.validate().map_err(invalid)?;
        match bundle.get(&Format::Files) {
            Some(data) => {
                let list = FileList::decode(data).map_err(invalid)?;
                if list.items.len() != files.len()
                    || list
                        .items
                        .iter()
                        .zip(&files)
                        .any(|(item, file)| *item != file.item())
                {
                    return Err(Error(libdunit::EINVAL));
                }
            }
            None if !files.is_empty() => return Err(Error(libdunit::EINVAL)),
            None => {}
        }
        Ok(Self { bundle, files })
    }
    pub fn text_value(&self) -> Option<&str> {
        self.bundle
            .get(&Format::Text)
            .and_then(|bytes| core::str::from_utf8(bytes).ok())
    }
    pub fn offer(&self, actions: u32, preferred: Action) -> NativeResult<Offer> {
        let offer = Offer {
            formats: self
                .bundle
                .representations
                .iter()
                .map(|r| r.format.clone())
                .collect(),
            actions,
            preferred,
        };
        offer.validate().map_err(invalid)?;
        Ok(offer)
    }
}
