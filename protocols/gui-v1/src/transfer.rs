//! Negotiated XFR1 data transfer. These packets never alias surface CAP1 imports.
use alloc::{string::String, vec::Vec};
pub mod broker;

pub const MAGIC: u32 = u32::from_le_bytes(*b"XFR1");
pub const MESSAGE_LEN: usize = 32;
pub const MAX_FORMATS: usize = 16;
pub const MAX_MIME_BYTES: usize = 127;
pub const MAX_BLOB_BYTES: usize = 16 * 1024 * 1024;
pub const BLOB_HEADER_LEN: usize = 12;
pub const MAX_BODY_BYTES: usize = MAX_BLOB_BYTES - BLOB_HEADER_LEN;
pub const MAX_IN_FLIGHT_BYTES: usize = 2 * MAX_BLOB_BYTES;
pub const MAX_FILE_ITEMS: usize = 256;
pub const MAX_PATH_BYTES: usize = 4096;
pub const ALL_FORMATS: u32 = u32::MAX;
pub const ACTIONS: u32 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Malformed,
    TooLarge,
    Unsupported,
}
pub type Result<T> = core::result::Result<T, Error>;

#[path = "transfer/values.rs"]
mod values;
pub use values::*;
impl Kind {
    pub const fn is_request(self) -> bool {
        (self as u16) < 0x8000
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Message {
    pub kind: Kind,
    pub request: u64,
    pub transfer: u64,
    pub arg0: u32,
    pub arg1: u32,
}
impl Message {
    pub fn encode(self) -> [u8; MESSAGE_LEN] {
        let mut b = [0; MESSAGE_LEN];
        b[..4].copy_from_slice(&MAGIC.to_le_bytes());
        b[4..6].copy_from_slice(&(self.kind as u16).to_le_bytes());
        b[8..16].copy_from_slice(&self.request.to_le_bytes());
        b[16..24].copy_from_slice(&self.transfer.to_le_bytes());
        b[24..28].copy_from_slice(&self.arg0.to_le_bytes());
        b[28..32].copy_from_slice(&self.arg1.to_le_bytes());
        b
    }
    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() != MESSAGE_LEN || b[..4] != MAGIC.to_le_bytes() || b[6..8] != [0; 2] {
            return Err(Error::Malformed);
        }
        let kind = Kind::from_u16(u16::from_le_bytes(
            b[4..6].try_into().map_err(|_| Error::Malformed)?,
        ))
        .ok_or(Error::Unsupported)?;
        let m = Self {
            kind,
            request: u64::from_le_bytes(b[8..16].try_into().map_err(|_| Error::Malformed)?),
            transfer: u64::from_le_bytes(b[16..24].try_into().map_err(|_| Error::Malformed)?),
            arg0: u32::from_le_bytes(b[24..28].try_into().map_err(|_| Error::Malformed)?),
            arg1: u32::from_le_bytes(b[28..32].try_into().map_err(|_| Error::Malformed)?),
        };
        if kind.is_request() && m.request == 0 {
            return Err(Error::Malformed);
        }
        Ok(m)
    }
    pub fn is_message(b: &[u8]) -> bool {
        b.get(..4) == Some(MAGIC.to_le_bytes().as_slice())
    }
}

/// A shared blob preserves the control packet's semantic arguments while its
/// outer arg0/arg1 carry the attached handle and bounded byte length.
pub fn encode_blob(arg0: u32, arg1: u32, bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.is_empty() || bytes.len() > MAX_BODY_BYTES {
        return Err(Error::TooLarge);
    }
    let mut body = Vec::with_capacity(BLOB_HEADER_LEN + bytes.len());
    body.extend_from_slice(b"XBD1");
    put(&mut body, arg0);
    put(&mut body, arg1);
    body.extend_from_slice(bytes);
    Ok(body)
}
pub fn decode_blob(bytes: &[u8]) -> Result<(u32, u32, &[u8])> {
    if bytes.len() <= BLOB_HEADER_LEN
        || bytes.len() > MAX_BLOB_BYTES
        || bytes.get(..4) != Some(b"XBD1")
    {
        return Err(Error::Malformed);
    }
    Ok((
        u32::from_le_bytes(bytes[4..8].try_into().map_err(|_| Error::Malformed)?),
        u32::from_le_bytes(bytes[8..12].try_into().map_err(|_| Error::Malformed)?),
        &bytes[BLOB_HEADER_LEN..],
    ))
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Format {
    Text,
    Files,
    Rgba,
    Custom(String),
}
impl Format {
    pub fn name(&self) -> &str {
        match self {
            Self::Text => TEXT_MIME,
            Self::Files => FILES_MIME,
            Self::Rgba => RGBA_MIME,
            Self::Custom(s) => s,
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        if !valid_mime(s) {
            return Err(Error::Malformed);
        }
        Ok(match s {
            TEXT_MIME => Self::Text,
            FILES_MIME => Self::Files,
            RGBA_MIME => Self::Rgba,
            _ => Self::Custom(String::from(s)),
        })
    }
    pub fn validate(&self) -> Result<()> {
        if Self::parse(self.name())? != *self {
            Err(Error::Malformed)
        } else {
            Ok(())
        }
    }
}
fn valid_mime(s: &str) -> bool {
    fn token(s: &str) -> bool {
        !s.is_empty()
            && s.bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"!#$&^_.+-".contains(&c))
    }
    if s.len() > MAX_MIME_BYTES {
        return false;
    }
    let mut parts = s.split(';');
    let Some((major, minor)) = parts.next().and_then(|v| v.split_once('/')) else {
        return false;
    };
    token(major)
        && token(minor)
        && parts.all(|p| p.split_once('=').is_some_and(|(k, v)| token(k) && token(v)))
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub formats: Vec<Format>,
    pub actions: u32,
    pub preferred: Action,
}
impl Offer {
    pub fn validate(&self) -> Result<()> {
        if self.formats.is_empty() || self.formats.len() > MAX_FORMATS {
            return Err(Error::TooLarge);
        }
        if self.actions == 0
            || self.actions & !ACTIONS != 0
            || self.actions & self.preferred as u32 == 0
        {
            return Err(Error::Malformed);
        }
        for (i, format) in self.formats.iter().enumerate() {
            format.validate()?;
            if self.formats[..i].contains(format) {
                return Err(Error::Malformed);
            }
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut b = Vec::new();
        b.extend_from_slice(b"XOF1");
        put(&mut b, self.actions);
        put(&mut b, self.preferred as u32);
        put(&mut b, self.formats.len() as u32);
        for f in &self.formats {
            string(&mut b, f.name());
        }
        Ok(b)
    }
    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b, b"XOF1")?;
        let actions = r.u32()?;
        let preferred = Action::from_u32(r.u32()?).ok_or(Error::Malformed)?;
        let count = r.count(MAX_FORMATS)?;
        let mut formats = Vec::new();
        for _ in 0..count {
            formats.push(Format::parse(r.string(MAX_MIME_BYTES)?)?)
        }
        r.end()?;
        let offer = Self {
            formats,
            actions,
            preferred,
        };
        offer.validate()?;
        Ok(offer)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Negotiated {
    pub format: usize,
    pub action: Action,
}
/// Target format order expresses target preference. A disallowed requested
/// action is rejected rather than silently downgraded (e.g. Move to Copy).
pub fn negotiate(offer: &Offer, target: &Offer, action: Action) -> Option<Negotiated> {
    offer.validate().ok()?;
    target.validate().ok()?;
    if offer.actions & target.actions & action as u32 == 0 {
        return None;
    }
    target
        .formats
        .iter()
        .find_map(|f| offer.formats.iter().position(|x| x == f))
        .map(|format| Negotiated { format, action })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Representation {
    pub format: Format,
    pub data: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    pub representations: Vec<Representation>,
}
impl Bundle {
    pub fn validate(&self) -> Result<()> {
        if self.representations.is_empty() || self.representations.len() > MAX_FORMATS {
            return Err(Error::TooLarge);
        }
        let mut total = 8usize;
        for (i, rep) in self.representations.iter().enumerate() {
            rep.format.validate()?;
            if self.representations[..i]
                .iter()
                .any(|r| r.format == rep.format)
            {
                return Err(Error::Malformed);
            }
            total = total
                .checked_add(rep.data.len())
                .and_then(|n| n.checked_add(8 + rep.format.name().len()))
                .ok_or(Error::TooLarge)?;
            if total > MAX_BODY_BYTES {
                return Err(Error::TooLarge);
            }
            match rep.format {
                Format::Text => {
                    core::str::from_utf8(&rep.data).map_err(|_| Error::Malformed)?;
                    if rep.data.contains(&0) {
                        return Err(Error::Malformed);
                    }
                }
                Format::Files => {
                    FileList::decode(&rep.data)?;
                }
                Format::Rgba => {
                    Rgba::decode(&rep.data)?;
                }
                Format::Custom(ref mime) if mime == URI_LIST_MIME => {
                    PathList::decode(&rep.data)?;
                }
                Format::Custom(_) => {}
            }
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut b = Vec::new();
        b.extend_from_slice(b"XDB1");
        put(&mut b, self.representations.len() as u32);
        for rep in &self.representations {
            string(&mut b, rep.format.name());
            put(&mut b, rep.data.len() as u32);
            b.extend_from_slice(&rep.data)
        }
        if b.len() > MAX_BLOB_BYTES {
            return Err(Error::TooLarge);
        }
        Ok(b)
    }
    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b, b"XDB1")?;
        let count = r.count(MAX_FORMATS)?;
        let mut representations = Vec::new();
        for _ in 0..count {
            let format = Format::parse(r.string(MAX_MIME_BYTES)?)?;
            let bytes = r.count(MAX_BLOB_BYTES)?;
            let data = r.bytes(bytes)?.to_vec();
            representations.push(Representation { format, data })
        }
        r.end()?;
        let b = Self { representations };
        b.validate()?;
        Ok(b)
    }
    pub fn get(&self, format: &Format) -> Option<&[u8]> {
        self.representations
            .iter()
            .find(|r| &r.format == format)
            .map(|r| r.data.as_slice())
    }
}
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    File = 1,
    Directory = 2,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileItem {
    pub path: String,
    pub capability: u32,
    pub kind: FileKind,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileList {
    pub items: Vec<FileItem>,
}
impl FileList {
    pub fn validate(&self) -> Result<()> {
        if self.items.is_empty() || self.items.len() > MAX_FILE_ITEMS {
            return Err(Error::TooLarge);
        }
        for (i, item) in self.items.iter().enumerate() {
            if item.capability == 0
                || !valid_path(&item.path)
                || self.items[..i]
                    .iter()
                    .any(|other| other.path == item.path || other.capability == item.capability)
            {
                return Err(Error::Malformed);
            }
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut b = Vec::new();
        b.extend_from_slice(b"XFL1");
        put(&mut b, self.items.len() as u32);
        for f in &self.items {
            put(&mut b, f.capability);
            put(&mut b, f.kind as u32);
            string(&mut b, &f.path)
        }
        Ok(b)
    }
    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b, b"XFL1")?;
        let count = r.count(MAX_FILE_ITEMS)?;
        let mut items = Vec::new();
        for _ in 0..count {
            let capability = r.u32()?;
            let kind = match r.u32()? {
                1 => FileKind::File,
                2 => FileKind::Directory,
                _ => return Err(Error::Malformed),
            };
            let path = String::from(r.string(MAX_PATH_BYTES)?);
            items.push(FileItem {
                path,
                capability,
                kind,
            })
        }
        r.end()?;
        let list = Self { items };
        list.validate()?;
        Ok(list)
    }
}
fn valid_path(s: &str) -> bool {
    s.len() <= MAX_PATH_BYTES
        && s.starts_with('/')
        && !s.starts_with("//")
        && !s.chars().any(char::is_control)
        && (s == "/"
            || !s[1..]
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == ".."))
}
/// Standard local file URIs for text receivers. They carry no file authority;
/// file receivers negotiate FILES_MIME and receive real READ capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathList {
    pub paths: Vec<String>,
}
impl PathList {
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.paths.is_empty() || self.paths.len() > MAX_FILE_ITEMS {
            return Err(Error::TooLarge);
        }
        let mut out = Vec::new();
        for (i, path) in self.paths.iter().enumerate() {
            if !valid_path(path) || self.paths[..i].contains(path) {
                return Err(Error::Malformed);
            }
            out.extend_from_slice(b"file://");
            for byte in path.bytes() {
                if uri_byte(byte) {
                    out.push(byte);
                } else {
                    out.push(b'%');
                    out.push(HEX[(byte >> 4) as usize]);
                    out.push(HEX[(byte & 15) as usize]);
                }
            }
            out.extend_from_slice(b"\r\n");
        }
        Ok(out)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_BODY_BYTES {
            return Err(Error::TooLarge);
        }
        let text = core::str::from_utf8(bytes).map_err(|_| Error::Malformed)?;
        let mut paths = Vec::new();
        for line in text.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let uri = line
                .strip_prefix("file://")
                .ok_or(Error::Unsupported)?
                .as_bytes();
            let mut path = Vec::new();
            let mut at = 0;
            while at < uri.len() {
                let b = uri[at];
                if b == b'%' {
                    let pair = uri.get(at + 1..at + 3).ok_or(Error::Malformed)?;
                    path.push(
                        unhex(pair[0])?.checked_mul(16).ok_or(Error::Malformed)? | unhex(pair[1])?,
                    );
                    at += 3;
                } else {
                    if !uri_byte(b) {
                        return Err(Error::Malformed);
                    }
                    path.push(b);
                    at += 1;
                }
                if path.len() > MAX_PATH_BYTES {
                    return Err(Error::TooLarge);
                }
            }
            let path = String::from_utf8(path).map_err(|_| Error::Malformed)?;
            if !valid_path(&path) || paths.contains(&path) {
                return Err(Error::Malformed);
            }
            paths.push(path);
            if paths.len() > MAX_FILE_ITEMS {
                return Err(Error::TooLarge);
            }
        }
        if paths.is_empty() {
            return Err(Error::Malformed);
        }
        Ok(Self { paths })
    }
}
const HEX: &[u8; 16] = b"0123456789ABCDEF";
fn uri_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"/-._~".contains(&b)
}
fn unhex(b: u8) -> Result<u8> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        _ => Err(Error::Malformed),
    }
}

/// One actual capability-scoped move request. Destination is a directory WRITE
/// capability owned by the broker after transfer; leaf is one path component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMove {
    pub item: u32,
    pub destination: u32,
    pub leaf: String,
}
impl FileMove {
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.item as usize >= MAX_FILE_ITEMS || self.destination == 0 || !valid_leaf(&self.leaf)
        {
            return Err(Error::Malformed);
        }
        let mut b = Vec::new();
        b.extend_from_slice(b"XMV1");
        put(&mut b, self.item);
        put(&mut b, self.destination);
        string(&mut b, &self.leaf);
        Ok(b)
    }
    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b, b"XMV1")?;
        let item = r.u32()?;
        let destination = r.u32()?;
        let leaf = String::from(r.string(MAX_PATH_BYTES)?);
        r.end()?;
        let value = Self {
            item,
            destination,
            leaf,
        };
        value.encode()?;
        Ok(value)
    }
}
fn valid_leaf(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && s != "."
        && s != ".."
        && !s.contains('/')
        && !s.chars().any(char::is_control)
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}
impl Rgba {
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut b = Vec::new();
        put(&mut b, self.width);
        put(&mut b, self.height);
        b.extend_from_slice(&self.pixels);
        Ok(b)
    }
    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() < 8 || b.len() > MAX_BLOB_BYTES {
            return Err(Error::Malformed);
        }
        let value = Self {
            width: u32::from_le_bytes(b[..4].try_into().map_err(|_| Error::Malformed)?),
            height: u32::from_le_bytes(b[4..8].try_into().map_err(|_| Error::Malformed)?),
            pixels: b[8..].to_vec(),
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<()> {
        let size = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|s| s.checked_mul(4))
            .ok_or(Error::TooLarge)?;
        if self.width == 0
            || self.height == 0
            || size > MAX_BLOB_BYTES - 8
            || self.pixels.len() != size
        {
            return Err(Error::Malformed);
        }
        Ok(())
    }
}

fn put(b: &mut Vec<u8>, n: u32) {
    b.extend_from_slice(&n.to_le_bytes())
}
fn string(b: &mut Vec<u8>, s: &str) {
    put(b, s.len() as u32);
    b.extend_from_slice(s.as_bytes())
}
struct Reader<'a> {
    bytes: &'a [u8],
}
impl<'a> Reader<'a> {
    fn new(b: &'a [u8], magic: &[u8; 4]) -> Result<Self> {
        if b.len() > MAX_BLOB_BYTES {
            return Err(Error::TooLarge);
        }
        if b.get(..4) != Some(magic.as_slice()) {
            return Err(Error::Malformed);
        }
        Ok(Self { bytes: &b[4..] })
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.bytes.len() {
            return Err(Error::Malformed);
        }
        let (value, rest) = self.bytes.split_at(n);
        self.bytes = rest;
        Ok(value)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.bytes(4)?.try_into().map_err(|_| Error::Malformed)?,
        ))
    }
    fn count(&mut self, max: usize) -> Result<usize> {
        let n = self.u32()? as usize;
        if n > max {
            return Err(Error::TooLarge);
        }
        Ok(n)
    }
    fn string(&mut self, max: usize) -> Result<&'a str> {
        let n = self.count(max)?;
        core::str::from_utf8(self.bytes(n)?).map_err(|_| Error::Malformed)
    }
    fn end(self) -> Result<()> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(Error::Malformed)
        }
    }
}
