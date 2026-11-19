#![no_std]
#![no_main]

extern crate alloc;

mod dhcp_runtime;
mod interface;
mod outbound;

use alloc::vec;
use dunit::{errno, process, time::Instant, Error, Result};
use dunit_net_core::{acd, dhcp};
use dunit_packet_abi::{Records, MAX_BATCH_LEN};
use libdunit::wait_set;
use netd::{wire_error, Device};
use dhcp_runtime::Dynamic;
use interface::{Config, Interface};
use outbound::Outbound;

const CONFIG_PATH: &str = "/system/boot-services/netd-ipv4.conf";
const LINK_PERIOD_MS: u64 = 1_000;

fn static_ipv4() -> Result<Option<Config>> {
    let text = match dunit::fs::read_to_string(CONFIG_PATH) {
        Ok(text) => text,
        Err(Error(errno::ENOENT)) => return Ok(None),
        Err(error) => return Err(error),
    };
    Config::static_address(&text).map(Some).ok_or(Error(errno::EINVAL))
}

fn clock_ms(started: Instant) -> Result<u64> {
    Ok(started.elapsed()?.as_millis() as u64)
}

fn network_error(_: dunit_net_core::Error) -> Error {
    Error(errno::EINVAL)
}

#[dunit::main]
fn main() -> Result<()> {
    let config = static_ipv4()?;
    let device = Device::receive_grant()?;
    let queue = device.open()?;
    let info = device.info()?;
    let started = Instant::now()?;
    let mut interface = Interface::new(info.mac, info.mtu as usize);
    let mut dynamic = if let Some(config) = config {
        interface.apply(config, false).map_err(network_error)?;
        dunit::println!("[NETD] packet queue ready with static IPv4")?;
        None
    } else {
        // Transaction uniqueness only. No cryptographic entropy is assumed.
        let ticks = dunit::system::stats().map_or(0, |stats| stats.uptime_ticks as u32);
        let seed = process::id() ^ ticks.rotate_left(13);
        let dynamic = Dynamic::new(info.mac, seed)?;
        dunit::println!("[NETD] packet queue ready for DHCPv4")?;
        Some(dynamic)
    };
    let mut link = info.link_up;
    let mut link_poll = LINK_PERIOD_MS;
    let mut rx = vec![0; MAX_BATCH_LEN];
    let mut tx = Outbound::new();

    loop {
        let now = clock_ms(started)?;
        if let Some(dynamic) = &mut dynamic {
            if now >= link_poll {
                let current = device.info()?.link_up;
                if current != link {
                    dynamic.reset(now, &mut interface, &mut tx)?;
                    link = current;
                }
                link_poll = now.saturating_add(LINK_PERIOD_MS);
            }
            if link {
                dynamic.tick(now, &mut interface, &mut tx)?;
            }
        }

        match queue.try_receive(&mut rx) {
            Ok(size) => {
                for frame in Records::new(&rx[..size]).map_err(wire_error)? {
                    if let Some(dynamic) = &mut dynamic {
                        if !link {
                            continue;
                        }
                        if dhcp::is_candidate(frame) {
                            dynamic.receive(frame, now, &mut interface, &mut tx)?;
                            continue;
                        }
                        if dynamic.observe(frame, now, &mut interface, &mut tx)? {
                            continue;
                        }
                    }
                    if let Some(config) = interface.config() {
                        if let Ok(Some(response)) = acd::reply_to_probe(info.mac, config.address, frame) {
                            tx.push(&response)?;
                            continue;
                        }
                    }
                    if let Some(peer) = interface.peer() {
                        if let Ok(Some(response)) = peer.receive(frame, now) {
                            tx.push(&response)?;
                        }
                    }
                }
            }
            Err(Error(errno::EAGAIN)) => {}
            Err(error) => return Err(error),
        }
        if link {
            if let Some(dynamic) = &mut dynamic {
                dynamic.drive_probe(clock_ms(started)?, &queue, &mut interface, &mut tx)?;
                dynamic.transmit(clock_ms(started)?, &mut interface, &mut tx)?;
            }
        }
        tx.flush(&queue)?;

        let write = !tx.is_empty() || dynamic.as_ref().is_some_and(Dynamic::waiting_for_probe_tx);
        let interests = wait_set::READ | if write { wait_set::WRITE } else { 0 };
        let timeout = if let Some(dynamic) = &dynamic {
            let next = if link { link_poll.min(dynamic.deadline()) } else { link_poll };
            let current = clock_ms(started)?;
            next.saturating_sub(current).saturating_mul(1_000_000)
        } else {
            wait_set::INFINITE
        };
        match queue.wait(interests, timeout) {
            Ok(()) | Err(Error(errno::EAGAIN)) => {}
            Err(error) => return Err(error),
        }
    }
}
