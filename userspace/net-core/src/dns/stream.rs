//! DNS over a connected byte stream uses a two-byte, network-order length.
use super::{Error, Vec};

pub const TCP_DNS_LIMIT: usize = 4096;
const PREFIX_BYTES: usize = 2;
const MIN_MESSAGE: usize = 12;

pub(super) fn encode(body: &[u8]) -> Result<Vec<u8>, Error> {
    if !(MIN_MESSAGE..=TCP_DNS_LIMIT).contains(&body.len()) {
        return Err(Error::Length);
    }
    let mut frame = Vec::with_capacity(PREFIX_BYTES + body.len());
    frame.extend_from_slice(&(body.len() as u16).to_be_bytes());
    frame.extend_from_slice(body);
    Ok(frame)
}

#[derive(Default)]
pub(super) struct ResponseFrame {
    header: [u8; PREFIX_BYTES],
    header_used: usize,
    body: Vec<u8>,
}

impl ResponseFrame {
    /// Accept exactly one message. Even a large input buffer cannot reserve
    /// memory until a complete, valid prefix has been received.
    pub(super) fn feed(&mut self, mut input: &[u8]) -> Result<Option<&[u8]>, Error> {
        let prefix = input.len().min(PREFIX_BYTES - self.header_used);
        self.header[self.header_used..self.header_used + prefix].copy_from_slice(&input[..prefix]);
        self.header_used += prefix;
        input = &input[prefix..];
        if self.header_used < PREFIX_BYTES {
            return Ok(None);
        }

        let size = usize::from(u16::from_be_bytes(self.header));
        if !(MIN_MESSAGE..=TCP_DNS_LIMIT).contains(&size) || input.len() > size - self.body.len() {
            return Err(Error::Length);
        }
        if self.body.capacity() == 0 {
            self.body = Vec::with_capacity(size);
        }
        self.body.extend_from_slice(input);
        Ok((self.body.len() == size).then_some(&self.body))
    }
}
