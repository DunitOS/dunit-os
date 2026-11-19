#![no_std]
#![no_main]

extern crate alloc;

mod address_probe;
mod dhcp_runtime;
mod interface;
mod outbound;
mod static_runtime;

use alloc::vec;
use dunit::{errno, process, time::Instant, Error, Result};
use dunit_net_core::{acd, dhcp};
use dunit_packet_abi::{Records, MAX_BATCH_LEN};
use libdunit::wait_set;
use netd::{wire_error, Device};
use dhcp_runtime::Dynamic;
use interface::{Config, Interface};
use outbound::Outbound;
use static_runtime::Static;

const CONFIG_PATH: &str = "/system/boot-services/netd-ipv4.conf";
const LINK_PERIOD_MS: u64 = 1_000;

enum Configuration {
    Dhcp(Dynamic),
    Static(Static),
}

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
    let mut tx = Outbound::new();
    // Discovery uniqueness only. No cryptographic entropy is assumed.
    let ticks = dunit::system::stats().map_or(0, |stats| stats.uptime_ticks as u32);
    let seed = process::id() ^ ticks.rotate_left(13);
    let mut mode = if let Some(config) = config {
        interface.prepare(config, false).map_err(network_error)?;
        let mut fixed = Static::new(config, info.mac, seed);
        fixed.reset(info.link_up, 0, &mut interface, &mut tx)?;
        dunit::println!("[NETD] packet queue ready with static IPv4")?;
        Configuration::Static(fixed)
    } else {
        let dynamic = Dynamic::new(info.mac, seed)?;
        dunit::println!("[NETD] packet queue ready for DHCPv4")?;
        Configuration::Dhcp(dynamic)
    };
    let mut link = info.link_up;
    let mut link_poll = LINK_PERIOD_MS;
    let mut rx = vec![0; MAX_BATCH_LEN];

    loop {
        let now = clock_ms(started)?;
        if now >= link_poll {
            let current = device.info()?.link_up;
            if current != link {
                match &mut mode {
                    Configuration::Dhcp(client) => client.reset(now, &mut interface, &mut tx)?,
                    Configuration::Static(fixed) => fixed.reset(current, now, &mut interface, &mut tx)?,
                }
                link = current;
            }
            link_poll = now.saturating_add(LINK_PERIOD_MS);
        }
        if link {
            if let Configuration::Dhcp(client) = &mut mode {
                client.tick(now, &mut interface, &mut tx)?;
            }
        }

        match queue.try_receive(&mut rx) {
            Ok(size) => {
                for frame in Records::new(&rx[..size]).map_err(wire_error)? {
                    if !link {
                        continue;
                    }
                    let handled = match &mut mode {
                        Configuration::Dhcp(client) if dhcp::is_candidate(frame) => {
                            client.receive(frame, now, &mut interface, &mut tx)?;
                            true
                        }
                        Configuration::Dhcp(client) => client.observe(frame, now, &mut interface, &mut tx)?,
                        Configuration::Static(fixed) => fixed.observe(frame, &mut interface, &mut tx)?,
                    };
                    if handled {
                        continue;
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
            let now = clock_ms(started)?;
            match &mut mode {
                Configuration::Dhcp(client) => {
                    client.drive_probe(now, &queue, &mut interface, &mut tx)?;
                    client.transmit(now, &mut interface, &mut tx)?;
                }
                Configuration::Static(fixed) => fixed.transmit(now, &queue, &mut interface)?,
            }
        }
        tx.flush(&queue)?;

        let (deadline, probe_write) = match &mode {
            Configuration::Dhcp(client) => (client.deadline(), client.waiting_for_probe_tx()),
            Configuration::Static(fixed) => (fixed.deadline(), fixed.waiting_for_tx()),
        };
        let interests = wait_set::READ | if !tx.is_empty() || probe_write { wait_set::WRITE } else { 0 };
        let next = if link { link_poll.min(deadline) } else { link_poll };
        let timeout = next.saturating_sub(clock_ms(started)?).saturating_mul(1_000_000);
        match queue.wait(interests, timeout) {
            Ok(()) | Err(Error(errno::EAGAIN)) => {}
            Err(error) => return Err(error),
        }
    }
}
