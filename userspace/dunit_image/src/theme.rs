//! Configured theme sources; one directory index is retained per loaded package.
use crate::{read, Error, Image};
use alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use dunit_icons::{
    archive::{Archive, ReadAt},
    safe_path,
    theme::{strings, Index},
};
struct File(usize);
impl File {
    fn open(path: &str) -> Result<Self, Error> {
        let fd = libdunit::open(path, libdunit::OPEN_READ);
        if fd < 0 {
            Err(if fd == -2 { Error::Missing } else { Error::Io })
        } else {
            Ok(Self(fd as usize))
        }
    }
}
impl Drop for File {
    fn drop(&mut self) {
        libdunit::close(self.0);
    }
}
impl ReadAt for File {
    fn len(&mut self) -> dunit_icons::Result<u64> {
        let n = libdunit::syscall3(libdunit::SYSCALL_SEEK, self.0, 0, 2);
        if n < 0 {
            Err(dunit_icons::Error::Io)
        } else {
            Ok(n as u64)
        }
    }
    fn read_at(&mut self, offset: u64, bytes: &mut [u8]) -> dunit_icons::Result<()> {
        let n = libdunit::syscall3(libdunit::SYSCALL_SEEK, self.0, offset as usize, 0);
        if n < 0 || n as u64 != offset {
            return Err(dunit_icons::Error::Io);
        }
        let mut p = 0;
        while p < bytes.len() {
            let end = (p + 4096).min(bytes.len());
            let n = libdunit::read(self.0, &mut bytes[p..end]);
            if n <= 0 {
                return Err(dunit_icons::Error::Io);
            }
            p += n as usize;
        }
        Ok(())
    }
}
impl From<dunit_icons::Error> for Error {
    fn from(e: dunit_icons::Error) -> Self {
        match e {
            dunit_icons::Error::Missing => Self::Missing,
            dunit_icons::Error::Io => Self::Io,
            dunit_icons::Error::Limit => Self::Limit,
            _ => Self::Format,
        }
    }
}
struct Source {
    directory: String,
    archive: Option<Archive<File>>,
    index: Option<Index>,
    aliases: Vec<(String, String)>,
}
/// A theme is selected by directory name under /assets/icons, or an absolute
/// manifest path. Only missing icons use the explicitly configured fallback.
pub struct IconTheme {
    sources: Vec<Source>,
}
impl IconTheme {
    pub fn open(theme: &str) -> Result<Self, Error> {
        let mut sources = Vec::new();
        let mut seen = Vec::new();
        let mut next = theme.to_string();
        loop {
            if seen.len() >= 8 || seen.contains(&next) {
                return Err(Error::Format);
            }
            seen.push(next.clone());
            let path = if next.starts_with('/') {
                next.clone()
            } else {
                if !safe_path(&next) || next.contains('/') {
                    return Err(Error::Format);
                }
                format!("/assets/icons/{}/theme.toml", next)
            };
            let data = match read(&path, 65536) {
                Ok(data) => data,
                Err(Error::Missing) if !next.starts_with('/') => {
                    sources.push(Source {
                        directory: format!("/assets/icons/{}/", next),
                        archive: None,
                        index: None,
                        aliases: Vec::new(),
                    });
                    break;
                }
                Err(e) => return Err(e),
            };
            let fields = strings(core::str::from_utf8(&data).map_err(|_| Error::Format)?)?;
            let mut archive_path = None;
            let mut directory = None;
            let mut aliases_path = None;
            let mut fallback = None;
            for (s, k, v) in fields {
                if s != "theme" {
                    return Err(Error::Format);
                }
                match k.as_str() {
                    "archive" => archive_path = Some(v),
                    "root" => directory = Some(v),
                    "aliases" => aliases_path = Some(v),
                    "fallback" => fallback = Some(v),
                    _ => return Err(Error::Format),
                }
            }
            let directory = directory.ok_or(Error::Format)?;
            let mut aliases = Vec::new();
            if let Some(path) = aliases_path {
                let bytes = read(&path, 65536)?;
                for (s, k, v) in strings(core::str::from_utf8(&bytes).map_err(|_| Error::Format)?)?
                {
                    if s != "icons" || !safe_path(&k) || !safe_path(&v) {
                        return Err(Error::Format);
                    }
                    aliases.push((k, v));
                }
            }
            let (archive, index) = if let Some(path) = archive_path {
                if !safe_path(directory.trim_end_matches('/')) {
                    return Err(Error::Format);
                }
                let mut a = Archive::open(File::open(&path)?)?;
                let bytes = a.read(
                    &format!("{}/index.theme", directory.trim_end_matches('/')),
                    65536,
                )?;
                let index = Index::parse(core::str::from_utf8(&bytes).map_err(|_| Error::Format)?)?;
                (Some(a), Some(index))
            } else {
                if !directory.starts_with('/') || !directory.ends_with('/') {
                    return Err(Error::Format);
                }
                (None, None)
            };
            sources.push(Source {
                directory,
                archive,
                index,
                aliases,
            });
            match fallback {
                Some(f) if !f.is_empty() => next = f,
                _ => break,
            }
        }
        Ok(Self { sources })
    }
    pub fn icon(&mut self, name: &str, side: u32) -> Result<Image, Error> {
        if name.starts_with('/') {
            return crate::load(name, side, side);
        }
        if !safe_path(name) {
            return Err(Error::Format);
        }
        for source in &mut self.sources {
            let stem = source
                .aliases
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str())
                .unwrap_or(name);
            if let (Some(a), Some(index)) = (&mut source.archive, &source.index) {
                let root = source.directory.trim_end_matches('/');
                let paths = if stem.contains('/') {
                    vec![format!("{}/{}", root, stem)]
                } else {
                    index
                        .candidates(side)
                        .iter()
                        .map(|d| format!("{}/{}/{}.svg", root, d, stem))
                        .collect()
                };
                for path in paths {
                    if !a.contains(&path) {
                        continue;
                    }
                    let bytes = a.read(&path, 1024 * 1024)?;
                    return crate::svg(&bytes, side, side);
                }
            } else {
                match crate::icon(&source.directory, stem, side) {
                    Err(Error::Missing) => {}
                    result => return result,
                }
            }
        }
        Err(Error::Missing)
    }
}
