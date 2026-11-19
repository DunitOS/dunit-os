#![no_std]
#![no_main]

extern crate alloc;

mod interface;
mod outbound;

use alloc::vec;
use dunit::{errno, process, time::Instant, Error, Result};
use dunit_net_core::{
    dhcp::{self, Action, Client, Event, Lease, Transmission},
    udp::{Address, Send},
    Mac,
};
use dunit_packet_abi::{Records, MAX_BATCH_LEN};
use libdunit::wait_set;
use netd::{wire_error, Device};
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

fn network_error(_: dunit_net_core::Error) -> Error {
    Error(errno::EINVAL)
}

struct Dynamic {
    client: Client,
    pending: Option<Transmission>,
}

impl Dynamic {
    fn remove(&mut self, interface: &mut Interface, tx: &mut Outbound) -> Result<()> {
        self.pending = None;
        tx.clear();
        if interface.clear() {
            dunit::println!("[NETD] DHCP lease removed")?;
        }
        Ok(())
    }

    fn tick(&mut self, now: u64, interface: &mut Interface, tx: &mut Outbound) -> Result<()> {
        match self.client.poll(now).map_err(network_error)? {
            Action::Idle => Ok(()),
            Action::Send(message) => {
                self.pending = Some(message);
                Ok(())
            }
            Action::Expired => self.remove(interface, tx),
        }
    }

    fn receive(
        &mut self,
        frame: &[u8],
        now: u64,
        interface: &mut Interface,
        tx: &mut Outbound,
    ) -> Result<()> {
        match self.client.receive(frame, now) {
            Ok(Event::Bound(lease)) => {
                self.pending = None;
                let renewed = interface.config().is_some();
                match interface.apply(from_lease(lease), true) {
                    Ok(changed) => {
                        if changed {
                            tx.clear();
                        }
                        let [a, b, c, d] = lease.address;
                        let verb = if renewed { "renewed" } else { "bound" };
                        dunit::println!("[NETD] DHCP {} {}.{}.{}.{}", verb, a, b, c, d)
                    }
                    Err(_) => {
                        // The candidate never replaced the previous peer. Discard
                        // the unusable lease and avoid serving an unowned address.
                        self.client.reset(now);
                        self.remove(interface, tx)
                    }
                }
            }
            Ok(Event::Lost) => self.remove(interface, tx),
            Ok(Event::Offered) => {
                self.pending = None;
                Ok(())
            }
            Ok(Event::Ignored) | Err(_) => Ok(()),
        }
    }

    fn transmit(
        &mut self,
        mac: Mac,
        now: u64,
        interface: &mut Interface,
        tx: &mut Outbound,
    ) -> Result<()> {
        let Some(message) = self.pending.as_ref() else {
            return Ok(());
        };
        if message.destination == dhcp::LIMITED_BROADCAST {
            let frame = message.broadcast_frame(mac).map_err(network_error)?;
            self.pending = None;
            return tx.push(&frame);
        }
        let Some(peer) = interface.peer() else {
            self.pending = None;
            return Ok(());
        };
        let server = Address { ip: message.destination, port: dhcp::SERVER_PORT };
        match peer.udp_send(dhcp::CLIENT_PORT, Some(server), &message.payload, now) {
            Ok(Send::Sent(frame)) => {
                self.pending = None;
                tx.push(&frame)
            }
            Ok(Send::Neighbor(frame)) => tx.push(&frame),
            Ok(Send::Waiting) => Ok(()),
            Err(_) => {
                // The lease timer drives retries and eventually rebroadcasts.
                self.pending = None;
                Ok(())
            }
        }
    }
}

fn from_lease(lease: Lease) -> Config {
    Config {
        address: lease.address,
        mask: lease.mask,
        router: lease.router,
        dns: lease.dns,
    }
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
        let client = Client::new(info.mac, seed).map_err(network_error)?;
        dunit::println!("[NETD] packet queue ready for DHCPv4")?;
        Some(Dynamic { client, pending: None })
    };
    let mut link = info.link_up;
    let mut link_poll = LINK_PERIOD_MS;
    let mut rx = vec![0; MAX_BATCH_LEN];
    let mut tx = Outbound::new();

    loop {
        let now = started.elapsed()?.as_millis() as u64;
        if let Some(dynamic) = &mut dynamic {
            if now >= link_poll {
                let current = device.info()?.link_up;
                if current != link {
                    dynamic.client.reset(now);
                    dynamic.remove(&mut interface, &mut tx)?;
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
                dynamic.transmit(info.mac, now, &mut interface, &mut tx)?;
            }
        }
        tx.flush(&queue)?;

        let interests = wait_set::READ | if tx.is_empty() { 0 } else { wait_set::WRITE };
        let timeout = if let Some(dynamic) = &dynamic {
            let next = if link { link_poll.min(dynamic.client.deadline()) } else { link_poll };
            let current = started.elapsed()?.as_millis() as u64;
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
