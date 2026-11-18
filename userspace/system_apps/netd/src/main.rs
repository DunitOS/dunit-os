#![no_std]
#![no_main]

extern crate alloc;

use alloc::{vec, vec::Vec};
use dunit::{errno, time::Instant, Error, Result};
use dunit_net_core::{Ip, Peer};
use dunit_packet_abi::{encode_record, Records, MAX_BATCH_LEN};
use netd::{wire_error, Device};

const CONFIG_PATH: &str = "/system/boot-services/netd-ipv4.conf";

fn static_ipv4() -> Result<Option<(Ip, Ip)>> {
    let text = match dunit::fs::read_to_string(CONFIG_PATH) {
        Ok(text) => text,
        Err(Error(errno::ENOENT)) => return Ok(None),
        Err(error) => return Err(error),
    };
    parse_ipv4(&text).map(Some).ok_or(Error(errno::EINVAL))
}

fn parse_ipv4(text: &str) -> Option<(Ip, Ip)> {
    let (address, prefix) = text.trim().split_once('/')?;
    let mut ip = [0; 4];
    let mut components = address.split('.');
    for octet in &mut ip {
        *octet = components.next()?.parse().ok()?;
    }
    if components.next().is_some() {
        return None;
    }
    let prefix = prefix.parse::<u32>().ok()?;
    if !(1..=32).contains(&prefix) {
        return None;
    }
    let mask = u32::MAX << (32 - prefix);
    Some((ip, mask.to_be_bytes()))
}

#[dunit::main]
fn main() -> Result<()> {
    let config = static_ipv4()?;
    let device = Device::receive_grant()?;
    let info = device.info()?;
    let mut peer = config
        .map(|(ip, mask)| Peer::new(info.mac, ip, mask, info.mtu as usize))
        .transpose()
        .map_err(|_| Error(errno::EINVAL))?;
    let queue = device.open()?;
    let started = Instant::now()?;
    let mut rx = vec![0; MAX_BATCH_LEN];
    let mut tx: Vec<u8> = vec![0; MAX_BATCH_LEN];
    if config.is_some() {
        dunit::println!("[NETD] packet queue ready with static IPv4")?;
    } else {
        dunit::println!("[NETD] packet queue ready without IPv4 configuration")?;
    }
    loop {
        let size = queue.receive(&mut rx)?;
        let batch = rx.get(..size).ok_or(Error(errno::EIO))?;
        let mut written = 0;
        let now = started.elapsed()?.as_millis() as u64;
        for frame in Records::new(batch).map_err(wire_error)? {
            if let Some(peer) = &mut peer {
                if let Ok(Some(response)) = peer.receive(frame, now) {
                    written += encode_record(&response, &mut tx[written..]).map_err(wire_error)?;
                }
            }
        }
        queue.send(&tx[..written])?;
    }
}
