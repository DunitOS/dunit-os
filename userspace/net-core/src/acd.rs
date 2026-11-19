//! IPv4 address conflict detection for a single candidate address (RFC 5227).
//! The interface owner commits or withdraws its address and owns any retry policy.
use crate::{valid_ip, valid_mac, Arp, Error, Ethernet, Ip, Mac, BROADCAST};
use alloc::vec::Vec;

const UNSPECIFIED: Ip = [0; 4];
const PROBES: u8 = 3;
const PROBE_WAIT_MS: u64 = 1_000;
const PROBE_MIN_MS: u64 = 1_000;
const PROBE_MAX_MS: u64 = 2_000;
const ANNOUNCE_WAIT_MS: u64 = 2_000;
const ANNOUNCE_INTERVAL_MS: u64 = 2_000;

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Idle,
    Probe(Vec<u8>),
    /// The quiet interval passed. Publish the address and send the first announcement.
    Claim(Vec<u8>),
    Announce(Vec<u8>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Probing(u8),
    Settling,
    Announcing,
    Complete,
}

/// One probe schedule. The caller can apply an action to a copy and retain
/// that copy once its frame has been accepted by the transmit queue.
#[derive(Clone, Copy, Debug)]
pub struct Probe {
    mac: Mac,
    address: Ip,
    phase: Phase,
    clock: u64,
    next: u64,
    random: u32,
}

impl Probe {
    pub fn new(mac: Mac, address: Ip, seed: u32, now_ms: u64) -> Result<Self, Error> {
        if !valid_mac(mac) || !valid_ip(address) || address[0] == 127 {
            return Err(Error::Address);
        }
        let mut probe = Self {
            mac,
            address,
            phase: Phase::Probing(0),
            clock: now_ms,
            next: now_ms,
            random: seed.max(1),
        };
        probe.next = now_ms.saturating_add(probe.random_delay(0, PROBE_WAIT_MS));
        Ok(probe)
    }

    pub fn deadline(&self) -> Option<u64> {
        (self.phase != Phase::Complete).then_some(self.next)
    }

    /// Check a received frame for a conflicting claim by another Ethernet
    /// station. While tentative, another station probing the same address
    /// also conflicts. During ownership only a sender IP claim conflicts.
    pub fn observe(&self, bytes: &[u8]) -> bool {
        let Ok(Some((frame, arp))) = received_arp(self.mac, bytes) else {
            return false;
        };
        arp.sender_ip == self.address
            || (matches!(self.phase, Phase::Probing(_) | Phase::Settling)
                && arp.operation == 1
                && frame.destination == BROADCAST
                && arp.sender_ip == UNSPECIFIED
                && arp.target_ip == self.address)
    }

    /// Advance at most one due action. A late poll spaces future actions from
    /// the actual send time instead of replaying missed sends back to back.
    pub fn poll(&mut self, now_ms: u64) -> Result<Action, Error> {
        if now_ms < self.clock {
            return Err(Error::Header);
        }
        self.clock = now_ms;
        if now_ms < self.next {
            return Ok(Action::Idle);
        }
        match self.phase {
            Phase::Probing(sent) => {
                let frame = self.frame(UNSPECIFIED)?;
                if sent + 1 == PROBES {
                    self.phase = Phase::Settling;
                    self.next = now_ms.saturating_add(ANNOUNCE_WAIT_MS);
                } else {
                    self.phase = Phase::Probing(sent + 1);
                    self.next =
                        now_ms.saturating_add(self.random_delay(PROBE_MIN_MS, PROBE_MAX_MS));
                }
                Ok(Action::Probe(frame))
            }
            Phase::Settling => {
                let frame = self.frame(self.address)?;
                self.phase = Phase::Announcing;
                self.next = now_ms.saturating_add(ANNOUNCE_INTERVAL_MS);
                Ok(Action::Claim(frame))
            }
            Phase::Announcing => {
                let frame = self.frame(self.address)?;
                self.phase = Phase::Complete;
                Ok(Action::Announce(frame))
            }
            Phase::Complete => Ok(Action::Idle),
        }
    }

    fn frame(&self, sender_ip: Ip) -> Result<Vec<u8>, Error> {
        let payload = Arp {
            operation: 1,
            sender_mac: self.mac,
            sender_ip,
            target_mac: [0; 6],
            target_ip: self.address,
        }
        .encode_framing();
        Ethernet::encode(BROADCAST, self.mac, 0x0806, &payload)
    }

    fn random_delay(&mut self, minimum: u64, maximum: u64) -> u64 {
        let mut random = self.random;
        random ^= random << 13;
        random ^= random >> 17;
        random ^= random << 5;
        self.random = random;
        minimum + u64::from(random) % (maximum - minimum + 1)
    }
}

/// Reply for an already configured interface when a foreign Ethernet station
/// probes the active address. This reply does not update the neighbor cache.
pub fn reply_to_probe(
    mac: Mac,
    active_address: Ip,
    bytes: &[u8],
) -> Result<Option<Vec<u8>>, Error> {
    if !valid_mac(mac) || !valid_ip(active_address) || active_address[0] == 127 {
        return Err(Error::Address);
    }
    let Some((frame, arp)) = received_arp(mac, bytes)? else {
        return Ok(None);
    };
    if frame.destination != BROADCAST
        || arp.operation != 1
        || arp.sender_ip != UNSPECIFIED
        || arp.target_ip != active_address
    {
        return Ok(None);
    }
    let response = Arp {
        operation: 2,
        sender_mac: mac,
        sender_ip: active_address,
        target_mac: frame.source,
        target_ip: UNSPECIFIED,
    }
    .encode_framing();
    Ethernet::encode(frame.source, mac, 0x0806, &response).map(Some)
}

fn received_arp(mac: Mac, bytes: &[u8]) -> Result<Option<(Ethernet<'_>, Arp)>, Error> {
    let frame = Ethernet::parse(bytes)?;
    if frame.kind != 0x0806
        || (frame.destination != BROADCAST && frame.destination != mac)
        || frame.source == mac
    {
        return Ok(None);
    }
    let arp = Arp::parse_framing(frame.payload)?;
    let valid_sender =
        valid_ip(arp.sender_ip) || (arp.operation == 1 && arp.sender_ip == UNSPECIFIED);
    let valid_target = valid_ip(arp.target_ip)
        || (arp.operation == 2
            && arp.target_ip == UNSPECIFIED
            && frame.destination == mac
            && arp.target_mac == mac);
    if arp.sender_mac != frame.source || !valid_sender || !valid_target {
        return Err(Error::Address);
    }
    Ok(Some((frame, arp)))
}

#[cfg(test)]
mod tests;
