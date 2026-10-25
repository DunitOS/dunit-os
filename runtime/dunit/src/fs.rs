//! Owned files over native descriptors. Drop closes once; close reports errors.
use crate::{
    error::checked,
    io::{Read, Write},
    Error, Result,
};
use alloc::{string::String, vec::Vec};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    File,
    Directory,
    Device,
}
#[derive(Debug, Clone, Copy)]
pub struct Metadata {
    pub file_type: FileType,
    pub len: u64,
}
fn metadata_from(stat: libdunit::FileStat) -> Result<Metadata> {
    let file_type = match stat.file_type {
        libdunit::FILE_TYPE_FILE => FileType::File,
        libdunit::FILE_TYPE_DIRECTORY => FileType::Directory,
        libdunit::FILE_TYPE_DEVICE => FileType::Device,
        _ => return Err(Error(libdunit::EINVAL)),
    };
    Ok(Metadata {
        file_type,
        len: stat.size as u64,
    })
}
#[derive(Default, Debug, Clone)]
pub struct OpenOptions {
    read: bool,
    write: bool,
    append: bool,
    create: bool,
    truncate: bool,
}
impl OpenOptions {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn read(&mut self, value: bool) -> &mut Self {
        self.read = value;
        self
    }
    pub fn write(&mut self, value: bool) -> &mut Self {
        self.write = value;
        self
    }
    pub fn append(&mut self, value: bool) -> &mut Self {
        self.append = value;
        self
    }
    pub fn create(&mut self, value: bool) -> &mut Self {
        self.create = value;
        self
    }
    pub fn truncate(&mut self, value: bool) -> &mut Self {
        self.truncate = value;
        self
    }
    pub fn open(&self, path: &str) -> Result<File> {
        if (!self.read && !self.write && !self.append)
            || ((self.create || self.truncate) && !self.write && !self.append)
        {
            return Err(Error(libdunit::EINVAL));
        }
        let mut flags = 0;
        for (enabled, bit) in [
            (self.read, libdunit::OPEN_READ),
            (self.write || self.append, libdunit::OPEN_WRITE),
            (self.append, libdunit::OPEN_APPEND),
            (self.create, libdunit::OPEN_CREATE),
            (self.truncate, libdunit::OPEN_TRUNC),
        ] {
            if enabled {
                flags |= bit;
            }
        }
        Ok(File {
            fd: Some(checked(libdunit::open(path, flags))?),
        })
    }
}
pub enum SeekFrom {
    Start(u64),
    Current(i64),
    End(i64),
}
#[derive(Debug)]
pub struct File {
    fd: Option<usize>,
}
impl File {
    pub fn open(path: &str) -> Result<Self> {
        OpenOptions::new().read(true).open(path)
    }
    pub fn create(path: &str) -> Result<Self> {
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
    }
    pub fn metadata(&self) -> Result<Metadata> {
        let mut stat = libdunit::FileStat::default();
        checked(libdunit::syscall2(
            libdunit::SYSCALL_FSTAT,
            self.fd.unwrap(),
            &mut stat as *mut _ as usize,
        ))?;
        metadata_from(stat)
    }
    pub fn seek(&mut self, from: SeekFrom) -> Result<u64> {
        let (offset, whence) = match from {
            SeekFrom::Start(offset) => (
                i64::try_from(offset).map_err(|_| Error(libdunit::EINVAL))?,
                0,
            ),
            SeekFrom::Current(offset) => (offset, 1),
            SeekFrom::End(offset) => (offset, 2),
        };
        checked(libdunit::syscall3(
            libdunit::SYSCALL_SEEK,
            self.fd.unwrap(),
            offset as usize,
            whence,
        ))
        .map(|position| position as u64)
    }
    pub fn sync_all(&self) -> Result<()> {
        checked(libdunit::fsync(self.fd.unwrap())).map(|_| ())
    }
    pub fn close(mut self) -> Result<()> {
        checked(libdunit::close(self.fd.take().unwrap())).map(|_| ())
    }
}
impl Drop for File {
    fn drop(&mut self) {
        if let Some(fd) = self.fd.take() {
            libdunit::close(fd);
        }
    }
}
impl Read for File {
    fn read(&mut self, output: &mut [u8]) -> Result<usize> {
        checked(libdunit::read(self.fd.unwrap(), output))
    }
}
impl Write for File {
    fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        checked(libdunit::write(self.fd.unwrap(), bytes))
    }
}
pub fn read(path: &str) -> Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    file.close()?;
    Ok(bytes)
}
pub fn read_to_string(path: &str) -> Result<String> {
    String::from_utf8(read(path)?).map_err(|_| Error(libdunit::EINVAL))
}
pub fn write(path: &str, bytes: &[u8]) -> Result<()> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.close()
}
pub fn metadata(path: &str) -> Result<Metadata> {
    let mut stat = libdunit::FileStat::default();
    checked(libdunit::stat(path, &mut stat))?;
    metadata_from(stat)
}
pub fn remove_file(path: &str) -> Result<()> {
    checked(libdunit::unlink(path)).map(|_| ())
}
pub fn rename(from: &str, to: &str) -> Result<()> {
    checked(libdunit::rename(from, to)).map(|_| ())
}
pub fn create_dir(path: &str) -> Result<()> {
    checked(libdunit::mkdir(path)).map(|_| ())
}
pub fn current_dir() -> Result<String> {
    let mut buffer = alloc::vec![0; 256];
    loop {
        let result = libdunit::getcwd(&mut buffer);
        if result == libdunit::ENAMETOOLONG {
            buffer.resize(
                buffer.len().checked_mul(2).ok_or(Error(libdunit::ENOMEM))?,
                0,
            );
        } else {
            let size = checked(result)?;
            buffer.truncate(size);
            return String::from_utf8(buffer).map_err(|_| Error(libdunit::EINVAL));
        }
    }
}
pub fn set_current_dir(path: &str) -> Result<()> {
    checked(libdunit::chdir(path)).map(|_| ())
}
