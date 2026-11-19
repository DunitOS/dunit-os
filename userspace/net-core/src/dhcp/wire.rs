//! BOOTP fixed header and RFC 2132 options used by the DHCPv4 client.
use super::{usable_ip, validate_config, Lease, Offer, LIMITED_BROADCAST};
use crate::udp::MAX_PAYLOAD;
use crate::{put, word, Error, Ip, Mac};
use alloc::vec::Vec;

const OPTIONS: usize = 240;
const MIN_REQUEST: usize = 300;
const MAX_OPTIONS: usize = 128;
const MAGIC: [u8; 4] = [99, 130, 83, 99];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(super) enum Kind {
    Discover = 1,
    Offer = 2,
    Request = 3,
    Decline = 4,
    Ack = 5,
    Nak = 6,
}

pub(super) struct Reply {
    pub kind: Kind,
    pub xid: u32,
    pub mac: Mac,
    pub client: Ip,
    pub address: Ip,
    pub server: Ip,
    options: Options,
}

#[derive(Default)]
struct Options {
    kind: Option<Kind>,
    mask: Option<Ip>,
    router: Option<Ip>,
    dns: Option<[Option<Ip>; 2]>,
    lease: Option<u32>,
    server: Option<Ip>,
    renew: Option<u32>,
    rebind: Option<u32>,
    client_id: bool,
}

impl Reply {
    pub fn lease(&self, now: u64, previous: Option<Lease>) -> Result<Lease, Error> {
        let mask = self
            .options
            .mask
            .or(previous.map(|lease| lease.mask))
            .ok_or(Error::Header)?;
        let router = self
            .options
            .router
            .or(previous.and_then(|lease| lease.router));
        let dns = self
            .options
            .dns
            .or(previous.map(|lease| lease.dns))
            .unwrap_or([None; 2]);
        validate_config(self.address, mask, router)?;
        let seconds = self.options.lease.ok_or(Error::Header)?;
        let (renew_at, rebind_at, expires_at) = if seconds == u32::MAX {
            (u64::MAX, u64::MAX, u64::MAX)
        } else {
            let duration_ms = u64::from(seconds) * 1000;
            let renew_ms = self
                .options
                .renew
                .map_or(duration_ms / 2, |seconds| u64::from(seconds) * 1000);
            let rebind_ms = self
                .options
                .rebind
                .map_or(duration_ms * 7 / 8, |seconds| u64::from(seconds) * 1000);
            if !(renew_ms > 0 && renew_ms < rebind_ms && rebind_ms < duration_ms) {
                return Err(Error::Header);
            }
            let expires_at = now.checked_add(duration_ms).ok_or(Error::Header)?;
            (now + renew_ms, now + rebind_ms, expires_at)
        };
        Ok(Lease {
            address: self.address,
            mask,
            router,
            dns,
            server: self.server,
            renew_at,
            rebind_at,
            expires_at,
        })
    }
}

fn unique<T>(field: &mut Option<T>, value: T) -> Result<(), Error> {
    if field.replace(value).is_some() {
        return Err(Error::Header);
    }
    Ok(())
}

fn fixed<const N: usize>(value: &[u8]) -> Result<[u8; N], Error> {
    value.try_into().map_err(|_| Error::Header)
}

fn address_list(value: &[u8]) -> Result<[Option<Ip>; 2], Error> {
    if value.is_empty() || value.len() % 4 != 0 {
        return Err(Error::Header);
    }
    let mut addresses = [None; 2];
    for (index, bytes) in value.chunks_exact(4).enumerate() {
        let address = fixed(bytes)?;
        if !usable_ip(address) {
            return Err(Error::Address);
        }
        if let Some(slot) = addresses.get_mut(index) {
            *slot = Some(address);
        }
    }
    Ok(addresses)
}

fn parse_options(mut bytes: &[u8], mac: Mac) -> Result<Options, Error> {
    let mut options = Options::default();
    let mut count = 0;
    while let Some((&tag, remainder)) = bytes.split_first() {
        bytes = remainder;
        if tag == 0 {
            continue;
        }
        if tag == 255 {
            return if bytes.iter().all(|&byte| byte == 0) {
                Ok(options)
            } else {
                Err(Error::Header)
            };
        }
        if count == MAX_OPTIONS {
            return Err(Error::Capacity);
        }
        count += 1;
        let (&len, rest) = bytes.split_first().ok_or(Error::Length)?;
        let value = rest.get(..usize::from(len)).ok_or(Error::Length)?;
        bytes = &rest[usize::from(len)..];
        match tag {
            1 => unique(&mut options.mask, fixed(value)?)?,
            3 => {
                let router = address_list(value)?[0].ok_or(Error::Address)?;
                unique(&mut options.router, router)?;
            }
            6 => unique(&mut options.dns, address_list(value)?)?,
            51 => unique(&mut options.lease, u32::from_be_bytes(fixed(value)?))?,
            52 => return Err(Error::Unsupported), // BOOTP file/sname option overload.
            53 => {
                let kind = match value {
                    [2] => Kind::Offer,
                    [5] => Kind::Ack,
                    [6] => Kind::Nak,
                    _ => return Err(Error::Protocol),
                };
                unique(&mut options.kind, kind)?;
            }
            54 => unique(&mut options.server, fixed(value)?)?,
            58 => unique(&mut options.renew, u32::from_be_bytes(fixed(value)?))?,
            59 => unique(&mut options.rebind, u32::from_be_bytes(fixed(value)?))?,
            61 => {
                if options.client_id || value.len() != 7 || value[0] != 1 || value[1..] != mac {
                    return Err(Error::Header);
                }
                options.client_id = true;
            }
            _ => {}
        }
    }
    Err(Error::Header) // DHCP END is mandatory even when the datagram ends here.
}

pub(super) fn reply(bytes: &[u8]) -> Result<Reply, Error> {
    if !(OPTIONS + 1..=MAX_PAYLOAD).contains(&bytes.len()) {
        return Err(Error::Length);
    }
    if bytes[..4] != [2, 1, 6, 0] // BOOTREPLY, Ethernet, MAC length, direct
        || word(bytes, 10) & 0x7fff != 0
        || bytes[24..28] != [0; 4] // giaddr: no relayed replies
        || bytes[236..240] != MAGIC
    {
        return Err(Error::Header);
    }
    let mac = fixed(&bytes[28..34])?;
    let options = parse_options(&bytes[OPTIONS..], mac)?;
    let server = options.server.ok_or(Error::Header)?;
    if !usable_ip(server) {
        return Err(Error::Address);
    }
    Ok(Reply {
        kind: options.kind.ok_or(Error::Header)?,
        xid: u32::from_be_bytes(fixed(&bytes[4..8])?),
        mac,
        client: fixed(&bytes[12..16])?,
        address: fixed(&bytes[16..20])?,
        server,
        options,
    })
}

fn option(out: &mut Vec<u8>, tag: u8, value: &[u8]) {
    out.push(tag);
    out.push(value.len() as u8);
    out.extend_from_slice(value);
}

pub(super) fn request(
    kind: Kind,
    xid: u32,
    mac: Mac,
    secs: u16,
    address: Ip,
    destination: Ip,
    chosen: Option<Offer>,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(MIN_REQUEST);
    out.resize(OPTIONS, 0);
    out[..4].copy_from_slice(&[1, 1, 6, 0]); // BOOTREQUEST, Ethernet, MAC length
    out[4..8].copy_from_slice(&xid.to_be_bytes());
    put(&mut out, 8, secs);
    if kind != Kind::Decline && destination == LIMITED_BROADCAST && address == [0; 4] {
        put(&mut out, 10, 0x8000);
    }
    out[12..16].copy_from_slice(&address);
    out[28..34].copy_from_slice(&mac);
    out[236..240].copy_from_slice(&MAGIC);
    option(&mut out, 53, &[kind as u8]);
    if let Some(offer) = chosen {
        option(&mut out, 50, &offer.address);
        option(&mut out, 54, &offer.server);
    }
    let mut client_id = [1; 7];
    client_id[1..].copy_from_slice(&mac);
    option(&mut out, 61, &client_id);
    if kind != Kind::Decline {
        option(&mut out, 55, &[1, 3, 6]);
        option(&mut out, 57, &(MAX_PAYLOAD as u16).to_be_bytes());
    }
    out.push(255);
    out.resize(MIN_REQUEST, 0);
    out
}
