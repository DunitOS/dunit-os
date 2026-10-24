use crate::{safe_path, Error, Result};
use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};
/// Random access lets the OS read only the directory and the selected icon.
pub trait ReadAt {
    fn len(&mut self) -> Result<u64>;
    fn read_at(&mut self, offset: u64, bytes: &mut [u8]) -> Result<()>;
}
#[derive(Clone)]
struct Entry {
    offset: u32,
    packed: u32,
    size: u32,
    crc: u32,
    method: u16,
    flags: u16,
}
pub struct Archive<R> {
    reader: R,
    entries: BTreeMap<String, Entry>,
    directory: u64,
}
fn u16at(b: &[u8], p: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        b.get(p..p + 2).ok_or(Error::Invalid)?.try_into().unwrap(),
    ))
}
fn u32at(b: &[u8], p: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(p..p + 4).ok_or(Error::Invalid)?.try_into().unwrap(),
    ))
}
impl<R: ReadAt> Archive<R> {
    pub fn open(mut reader: R) -> Result<Self> {
        let len = reader.len()?;
        if len < 22 {
            return Err(Error::Invalid);
        }
        let tail_len = len.min(65557) as usize;
        let mut tail = vec![0; tail_len];
        reader.read_at(len - tail_len as u64, &mut tail)?;
        let end = (0..=tail_len - 22)
            .rev()
            .find(|&p| {
                tail[p..p + 4] == *b"PK\x05\x06"
                    && p + 22 + u16at(&tail, p + 20).unwrap_or(0) as usize == tail_len
            })
            .ok_or(Error::Invalid)?;
        if u16at(&tail, end + 4)? != 0 || u16at(&tail, end + 6)? != 0 {
            return Err(Error::Unsupported);
        }
        let count = u16at(&tail, end + 10)? as usize;
        if count == 65535 || count != u16at(&tail, end + 8)? as usize {
            return Err(Error::Unsupported);
        }
        let size = u32at(&tail, end + 12)? as usize;
        let directory = u32at(&tail, end + 16)? as u64;
        if size > 8 * 1024 * 1024 {
            return Err(Error::Limit);
        }
        if directory + size as u64 != len - tail_len as u64 + end as u64 {
            return Err(Error::Invalid);
        }
        let mut data = vec![0; size];
        reader.read_at(directory, &mut data)?;
        let mut entries = BTreeMap::new();
        let mut p = 0;
        for _ in 0..count {
            if data.get(p..p + 4) != Some(b"PK\x01\x02") {
                return Err(Error::Invalid);
            }
            let flags = u16at(&data, p + 8)?;
            let method = u16at(&data, p + 10)?;
            if flags & !0x0808 != 0 || !matches!(method, 0 | 8) {
                return Err(Error::Unsupported);
            }
            let n = u16at(&data, p + 28)? as usize;
            let extra = u16at(&data, p + 30)? as usize;
            let comment = u16at(&data, p + 32)? as usize;
            if u16at(&data, p + 34)? != 0 {
                return Err(Error::Unsupported);
            }
            let mode = u32at(&data, p + 38)? >> 16;
            if mode & 0o170000 == 0o120000 {
                return Err(Error::Unsupported);
            }
            let name = core::str::from_utf8(data.get(p + 46..p + 46 + n).ok_or(Error::Invalid)?)
                .map_err(|_| Error::Invalid)?;
            // Directory records are allowed, but never used as file names.
            if !safe_path(name.trim_end_matches('/')) {
                return Err(Error::Invalid);
            }
            let entry = Entry {
                offset: u32at(&data, p + 42)?,
                packed: u32at(&data, p + 20)?,
                size: u32at(&data, p + 24)?,
                crc: u32at(&data, p + 16)?,
                method,
                flags,
            };
            if entry.offset as u64 + 30 > directory
                || entry.packed == u32::MAX
                || entry.size == u32::MAX
            {
                return Err(Error::Invalid);
            }
            if entries.insert(String::from(name), entry).is_some() {
                return Err(Error::Invalid);
            }
            p = p
                .checked_add(46 + n + extra + comment)
                .ok_or(Error::Invalid)?;
            if p > size {
                return Err(Error::Invalid);
            }
        }
        if p != size {
            return Err(Error::Invalid);
        }
        Ok(Self {
            reader,
            entries,
            directory,
        })
    }
    pub fn contains(&self, path: &str) -> bool {
        self.entries.contains_key(path)
    }
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }
    pub fn read(&mut self, path: &str, cap: usize) -> Result<Vec<u8>> {
        if !safe_path(path) {
            return Err(Error::Invalid);
        }
        let e = self.entries.get(path).ok_or(Error::Missing)?.clone();
        if e.size as usize > cap || e.packed as usize > cap {
            return Err(Error::Limit);
        }
        let mut h = [0; 30];
        self.reader.read_at(e.offset as u64, &mut h)?;
        if &h[..4] != b"PK\x03\x04" || u16at(&h, 6)? != e.flags || u16at(&h, 8)? != e.method {
            return Err(Error::Invalid);
        }
        if e.flags & 8 == 0
            && (u32at(&h, 14)? != e.crc || u32at(&h, 18)? != e.packed || u32at(&h, 22)? != e.size)
        {
            return Err(Error::Invalid);
        }
        let n = u16at(&h, 26)? as usize;
        let extra = u16at(&h, 28)? as u64;
        if n != path.len() {
            return Err(Error::Invalid);
        }
        let offset = e.offset as u64 + 30;
        let start = offset + n as u64 + extra;
        if start + e.packed as u64 > self.directory {
            return Err(Error::Invalid);
        }
        let mut name = vec![0; n];
        self.reader.read_at(offset, &mut name)?;
        if name != path.as_bytes() {
            return Err(Error::Invalid);
        }
        let mut packed = vec![0; e.packed as usize];
        self.reader.read_at(start, &mut packed)?;
        let bytes = if e.method == 0 {
            packed
        } else {
            miniz_oxide::inflate::decompress_to_vec_with_limit(&packed, e.size as usize)
                .map_err(|_| Error::Invalid)?
        };
        if bytes.len() != e.size as usize || crc32(&bytes) != e.crc {
            return Err(Error::Invalid);
        }
        Ok(bytes)
    }
}
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320u32.wrapping_mul(crc & 1));
        }
    }
    !crc
}
