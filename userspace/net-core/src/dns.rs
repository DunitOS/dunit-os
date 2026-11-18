//! Bounded DNS over the logical Peer's UDP endpoints. One transaction per
//! resolver, eight cached answers, 512-byte UDP messages and 32 records/message.
//! TC is Unsupported: TCP fallback and negative caching require later work.
use super::udp::{Address, Send};
use super::*;

pub const DNS_LIMIT: usize = 512;
pub const RETRY_MS: u64 = 1000;
pub const ATTEMPTS: u8 = 3;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    A,
    Aaaa,
}
impl Kind {
    fn wire(self) -> u16 {
        match self {
            Self::A => 1,
            Self::Aaaa => 28,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddressData {
    V4(Ip),
    V6([u8; 16]),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub address: AddressData,
    pub ttl: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub id: u16,
    pub name: Vec<u8>,
    pub kind: Kind,
}
fn normalize(name: &[u8]) -> Result<Vec<u8>, Error> {
    let name = name.strip_suffix(b".").unwrap_or(name);
    if name.is_empty() || name.len() > 253 {
        return Err(Error::Header);
    }
    for label in name.split(|c| *c == b'.') {
        if label.is_empty()
            || label.len() > 63
            || !label
                .iter()
                .all(|c| c.is_ascii_alphanumeric() || *c == b'-' || *c == b'_')
        {
            return Err(Error::Header);
        }
    }
    Ok(name.iter().map(u8::to_ascii_lowercase).collect())
}
fn write_name(out: &mut Vec<u8>, name: &[u8]) {
    for label in name.split(|c| *c == b'.') {
        out.push(label.len() as u8);
        out.extend_from_slice(label);
    }
    out.push(0);
}
/// Returns a normalized name, advancing the original cursor only past the
/// encoded name. Backward-only pointers and a 16-hop cap reject cycles.
fn read_name(b: &[u8], pos: &mut usize) -> Result<Vec<u8>, Error> {
    let mut cursor = *pos;
    let mut end = None;
    let mut jumps = 0;
    let mut name = Vec::new();
    loop {
        let len = *b.get(cursor).ok_or(Error::Length)?;
        if len & 0xc0 == 0xc0 {
            let low = *b.get(cursor + 1).ok_or(Error::Length)?;
            let target = (((len & 0x3f) as usize) << 8) | low as usize;
            if target < 12 || target >= cursor || jumps >= 16 {
                return Err(Error::Header);
            }
            end.get_or_insert(cursor + 2);
            cursor = target;
            jumps += 1;
            continue;
        }
        if len & 0xc0 != 0 {
            return Err(Error::Header);
        }
        cursor += 1;
        if len == 0 {
            *pos = end.unwrap_or(cursor);
            break;
        }
        let label = b.get(cursor..cursor + len as usize).ok_or(Error::Length)?;
        if !name.is_empty() {
            name.push(b'.');
        }
        name.extend_from_slice(label);
        if name.len() > 253 {
            return Err(Error::Length);
        }
        cursor += len as usize;
    }
    normalize(&name)
}
impl Question {
    pub fn new(id: u16, name: &[u8], kind: Kind) -> Result<Self, Error> {
        Ok(Self {
            id,
            name: normalize(name)?,
            kind,
        })
    }
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let name = normalize(&self.name)?;
        let mut b = alloc::vec![0; 12];
        put(&mut b, 0, self.id);
        put(&mut b, 2, 0x0100);
        put(&mut b, 4, 1);
        write_name(&mut b, &name);
        b.extend_from_slice(&self.kind.wire().to_be_bytes());
        b.extend_from_slice(&1u16.to_be_bytes());
        Ok(b)
    }
    pub fn parse(b: &[u8]) -> Result<Self, Error> {
        if !(12..=DNS_LIMIT).contains(&b.len()) {
            return Err(Error::Length);
        }
        if word(b, 2) != 0x0100 || word(b, 4) != 1 || b[6..12] != [0; 6] {
            return Err(Error::Header);
        }
        let mut pos = 12;
        let name = read_name(b, &mut pos)?;
        if pos + 4 != b.len() {
            return Err(Error::Length);
        }
        let kind = match word(b, pos) {
            1 => Kind::A,
            28 => Kind::Aaaa,
            _ => return Err(Error::Unsupported),
        };
        if word(b, pos + 2) != 1 {
            return Err(Error::Unsupported);
        }
        Self::new(word(b, 0), &name, kind)
    }
    /// Logical authoritative response helper, used by deterministic peers.
    pub fn response(&self, answer: &Answer) -> Result<Vec<u8>, Error> {
        if !matches!(
            (&answer.address, self.kind),
            (AddressData::V4(_), Kind::A) | (AddressData::V6(_), Kind::Aaaa)
        ) {
            return Err(Error::Protocol);
        }
        let mut b = self.encode()?;
        put(&mut b, 2, 0x8180);
        put(&mut b, 6, 1);
        b.extend_from_slice(&[0xc0, 12]);
        b.extend_from_slice(&self.kind.wire().to_be_bytes());
        b.extend_from_slice(&1u16.to_be_bytes());
        b.extend_from_slice(&answer.ttl.to_be_bytes());
        let data: &[u8] = match &answer.address {
            AddressData::V4(ip) => ip,
            AddressData::V6(ip) => ip,
        };
        b.extend_from_slice(&(data.len() as u16).to_be_bytes());
        b.extend_from_slice(data);
        Ok(b)
    }
}
struct Record {
    owner: Vec<u8>,
    kind: u16,
    ttl: u32,
    value: Value,
}
enum Value {
    Address(AddressData),
    Alias(Vec<u8>),
    Other,
}
pub fn parse_response(b: &[u8], question: &Question) -> Result<Answer, Error> {
    if !(12..=DNS_LIMIT).contains(&b.len()) {
        return Err(Error::Length);
    }
    if word(b, 0) != question.id {
        return Err(Error::Unsolicited);
    }
    let flags = word(b, 2);
    if flags & 0x8000 == 0 || flags & 0x7800 != 0 || flags & 0x0040 != 0 || word(b, 4) != 1 {
        return Err(Error::Header);
    }
    let mut pos = 12;
    let name = read_name(b, &mut pos)?;
    if pos + 4 > b.len() {
        return Err(Error::Length);
    }
    if name != question.name || word(b, pos) != question.kind.wire() || word(b, pos + 2) != 1 {
        return Err(Error::Unsolicited);
    }
    pos += 4;
    if flags & 0x0200 != 0 {
        return Err(Error::Unsupported);
    }
    let counts = [
        word(b, 6) as usize,
        word(b, 8) as usize,
        word(b, 10) as usize,
    ];
    let count = counts.iter().sum::<usize>();
    if count > 32 {
        return Err(Error::Capacity);
    }
    let mut records = Vec::new();
    for index in 0..count {
        let owner = read_name(b, &mut pos)?;
        if pos + 10 > b.len() {
            return Err(Error::Length);
        }
        let kind = word(b, pos);
        let class = word(b, pos + 2);
        let ttl = u32::from_be_bytes(b[pos + 4..pos + 8].try_into().unwrap());
        let len = word(b, pos + 8) as usize;
        pos += 10;
        let end = pos
            .checked_add(len)
            .filter(|e| *e <= b.len())
            .ok_or(Error::Length)?;
        let value = match (kind, class) {
            (1, 1) => {
                if len != 4 {
                    return Err(Error::Length);
                }
                Value::Address(AddressData::V4(b[pos..end].try_into().unwrap()))
            }
            (28, 1) => {
                if len != 16 {
                    return Err(Error::Length);
                }
                Value::Address(AddressData::V6(b[pos..end].try_into().unwrap()))
            }
            (5, 1) => {
                let alias = read_name(b, &mut pos)?;
                if pos != end {
                    return Err(Error::Length);
                }
                Value::Alias(alias)
            }
            _ => Value::Other,
        };
        pos = end;
        // Authority/additional records are parsed for framing, never answers.
        if index < counts[0] {
            records.push(Record {
                owner,
                kind,
                ttl,
                value,
            });
        }
    }
    if pos != b.len() {
        return Err(Error::Length);
    }
    match flags & 15 {
        0 => {}
        3 => return Err(Error::NameNotFound),
        _ => return Err(Error::Unsupported),
    }
    let mut target = question.name.clone();
    let mut visited = Vec::new();
    let mut ttl = u32::MAX;
    for _ in 0..=8 {
        if visited.contains(&target) {
            return Err(Error::Header);
        }
        visited.push(target.clone());
        if let Some(record) = records
            .iter()
            .find(|r| r.owner == target && r.kind == question.kind.wire())
        {
            if let Value::Address(address) = &record.value {
                return Ok(Answer {
                    address: address.clone(),
                    ttl: ttl.min(record.ttl),
                });
            }
        }
        let alias = records
            .iter()
            .find(|r| r.owner == target && r.kind == 5)
            .ok_or(Error::NameNotFound)?;
        if let Value::Alias(name) = &alias.value {
            target = name.clone();
            ttl = ttl.min(alias.ttl);
        } else {
            return Err(Error::Header);
        }
    }
    Err(Error::Capacity)
}
struct Cached {
    name: Vec<u8>,
    kind: Kind,
    answer: Answer,
    expires: u64,
}
struct Transaction {
    question: Question,
    attempts: u8,
    next: u64,
}
pub struct Resolver {
    server: Address,
    port: u16,
    pending: Option<Transaction>,
    cache: Vec<Cached>,
    last: u64,
    next_id: u16,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Poll {
    Frame(Vec<u8>),
    Waiting,
    Answer(Answer),
}
impl Resolver {
    /// Owns a connected ephemeral UDP port until close. Caller configures server.
    pub fn new(peer: &mut Peer, server: Address) -> Result<Self, Error> {
        let port = peer.udp_bind(0)?;
        if let Err(e) = peer.udp_connect(port, Some(server)) {
            peer.udp_close(port)?;
            return Err(e);
        }
        Ok(Self {
            server,
            port,
            pending: None,
            cache: Vec::new(),
            last: 0,
            next_id: 1,
        })
    }
    pub fn close(self, peer: &mut Peer) -> Result<(), Error> {
        peer.udp_close(self.port)
    }
    fn advance(&mut self, now: u64) -> Result<(), Error> {
        if now < self.last {
            return Err(Error::Header);
        }
        self.last = now;
        self.cache.retain(|c| c.expires > now);
        Ok(())
    }
    pub fn start(&mut self, name: &[u8], kind: Kind, now: u64) -> Result<Option<Answer>, Error> {
        self.advance(now)?;
        if self.pending.is_some() {
            return Err(Error::Capacity);
        }
        let question = Question::new(self.next_id, name, kind)?;
        if let Some(cached) = self
            .cache
            .iter()
            .find(|c| c.name == question.name && c.kind == kind)
        {
            let mut answer = cached.answer.clone();
            answer.ttl = ((cached.expires - now) / 1000) as u32;
            return Ok(Some(answer));
        }
        self.next_id = self.next_id.wrapping_add(1);
        self.pending = Some(Transaction {
            question,
            attempts: 0,
            next: now,
        });
        Ok(None)
    }
    pub fn poll(&mut self, peer: &mut Peer, now: u64) -> Result<Poll, Error> {
        self.advance(now)?;
        let tx = self.pending.as_mut().ok_or(Error::NotBound)?;
        // A packet arriving at/after the final deadline cannot rescue an expired query.
        if tx.attempts == ATTEMPTS && now >= tx.next {
            self.pending = None;
            return Err(Error::Timeout);
        }
        match peer.udp_recv(self.port) {
            Ok(packet) => {
                if packet.source != self.server || tx.attempts == 0 {
                    return Err(Error::Unsolicited);
                }
                let answer = match parse_response(&packet.data, &tx.question) {
                    Ok(a) => a,
                    Err(e) => {
                        if matches!(e, Error::NameNotFound | Error::Unsupported) {
                            self.pending = None;
                        }
                        return Err(e);
                    }
                };
                if answer.ttl != 0 {
                    if self.cache.len() == 8 {
                        self.cache.remove(0);
                    }
                    self.cache.push(Cached {
                        name: tx.question.name.clone(),
                        kind: tx.question.kind,
                        answer: answer.clone(),
                        expires: now.saturating_add(answer.ttl as u64 * 1000),
                    });
                }
                self.pending = None;
                return Ok(Poll::Answer(answer));
            }
            Err(Error::WouldBlock) => {}
            Err(e) => return Err(e),
        }
        if now < tx.next {
            return Ok(Poll::Waiting);
        }
        match peer.udp_send(self.port, None, &tx.question.encode()?, now)? {
            Send::Waiting => Ok(Poll::Waiting),
            Send::Neighbor(frame) => Ok(Poll::Frame(frame)),
            Send::Sent(frame) => {
                tx.attempts += 1;
                tx.next = now.saturating_add(RETRY_MS);
                Ok(Poll::Frame(frame))
            }
        }
    }
}
