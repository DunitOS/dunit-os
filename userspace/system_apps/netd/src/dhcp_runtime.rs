//! DHCP timers, tentative address probes and interface activation.
use dunit::Result;
use dunit_net_core::{
    dhcp::{self, Action, Client, Event, Lease, Transmission},
    udp::{Address, Send},
    Ip, Mac,
};
use netd::Queue;

use crate::{
    address_probe::AddressProbe,
    interface::{Config, Interface},
    lease_hint::LeaseHint,
    network_error,
    outbound::Outbound,
};

struct Candidate {
    config: Config,
    probe: AddressProbe,
}

pub struct Dynamic {
    mac: Mac,
    seed: u32,
    client: Client,
    remembered: Option<Ip>,
    hint: LeaseHint,
    pending: Option<Transmission>,
    candidate: Option<Candidate>,
}

impl Dynamic {
    pub fn new(mac: Mac, seed: u32, remembered: Option<Ip>) -> Result<Self> {
        let hint = LeaseHint::load(mac);
        Ok(Self {
            mac, seed,
            client: Client::new(mac, seed).map_err(network_error)?,
            remembered: remembered.or(hint.address()),
            hint,
            pending: None,
            candidate: None,
        })
    }

    pub fn deadline(&self) -> u64 {
        let probe = self.candidate.as_ref().and_then(|candidate| candidate.probe.deadline());
        self.client.deadline().min(probe.unwrap_or(u64::MAX))
    }

    pub fn waiting_for_probe_tx(&self) -> bool {
        self.candidate.as_ref().is_some_and(|candidate| candidate.probe.waiting_for_tx())
    }

    pub fn reset(&mut self, now: u64, interface: &mut Interface, tx: &mut Outbound) -> Result<()> {
        self.client.reset(now);
        self.remove(interface, tx)
    }

    fn remove(&mut self, interface: &mut Interface, tx: &mut Outbound) -> Result<()> {
        self.pending = None;
        self.candidate = None;
        tx.clear();
        if interface.clear() {
            dunit::println!("[NETD] DHCP lease removed")?;
        }
        Ok(())
    }

    pub fn tick(&mut self, now: u64, interface: &mut Interface, tx: &mut Outbound) -> Result<()> {
        // The first tick runs with an active link. A later link reset discards
        // the transaction along with any other address state.
        if let Some(address) = self.remembered.take() {
            self.client.revalidate(address, now).map_err(network_error)?;
        }
        match self.client.poll(now).map_err(network_error)? {
            Action::Idle => Ok(()),
            Action::Send(message) => {
                self.pending = Some(message);
                Ok(())
            }
            Action::Expired => self.withdraw(interface, tx),
        }
    }

    pub fn receive(
        &mut self,
        frame: &[u8],
        now: u64,
        interface: &mut Interface,
        tx: &mut Outbound,
    ) -> Result<()> {
        match self.client.receive(frame, now) {
            Ok(Event::Bound(lease)) => self.acknowledged(lease, now, interface, tx),
            Ok(Event::Lost) => self.withdraw(interface, tx),
            Ok(Event::Offered) => {
                self.pending = None;
                Ok(())
            }
            Ok(Event::Ignored) | Err(_) => Ok(()),
        }
    }

    fn withdraw(&mut self, interface: &mut Interface, tx: &mut Outbound) -> Result<()> {
        self.remove(interface, tx)?;
        if let Err(error) = self.hint.clear() {
            dunit::println!("[NETD] cannot clear DHCP hint: {:?}", error)?;
        }
        Ok(())
    }

    fn acknowledged(
        &mut self,
        lease: Lease,
        now: u64,
        interface: &mut Interface,
        tx: &mut Outbound,
    ) -> Result<()> {
        self.pending = None;
        let config = Config {
            address: lease.address, mask: lease.mask, router: lease.router, dns: lease.dns,
            routes: lease.routes,
        };
        if let Some(candidate) = &mut self.candidate {
            if candidate.config.address == config.address {
                candidate.config = config;
                if interface.config().is_some() {
                    return self.apply(config, "renewed", now, interface, tx);
                }
                return Ok(());
            }
        }

        self.remove(interface, tx)?;
        self.seed = self.seed.wrapping_add(0x9e3779b9);
        let probe = AddressProbe::new(self.mac, config.address, self.seed ^ now as u32, now)?;
        self.candidate = Some(Candidate { config, probe });
        Ok(())
    }

    fn apply(
        &mut self,
        config: Config,
        verb: &str,
        now: u64,
        interface: &mut Interface,
        tx: &mut Outbound,
    ) -> Result<()> {
        let changed = match interface.apply(config, true) {
            Ok(changed) => changed,
            Err(_) => return self.reset(now, interface, tx),
        };
        if changed {
            tx.clear();
        }
        if let Err(error) = self.hint.save(config.address) {
            dunit::println!("[NETD] cannot save DHCP hint: {:?}", error)?;
        }
        let [a, b, c, d] = config.address;
        dunit::println!("[NETD] DHCP {} {}.{}.{}.{}", verb, a, b, c, d)
    }

    pub fn observe(
        &mut self,
        frame: &[u8],
        now: u64,
        interface: &mut Interface,
        tx: &mut Outbound,
    ) -> Result<bool> {
        let Some(candidate) = self.candidate.as_ref() else {
            return Ok(false);
        };
        if !candidate.probe.observe(frame) {
            return Ok(false);
        }
        let [a, b, c, d] = candidate.config.address;
        let decline = self.client.decline(now).map_err(network_error)?;
        self.withdraw(interface, tx)?;
        self.pending = Some(decline);
        dunit::println!("[NETD] DHCP conflict {}.{}.{}.{}", a, b, c, d)?;
        Ok(true)
    }

    pub fn drive_probe(
        &mut self,
        now: u64,
        queue: &Queue,
        interface: &mut Interface,
        tx: &mut Outbound,
    ) -> Result<()> {
        if self.client.lease().is_some_and(|lease| now >= lease.expires_at) {
            return self.tick(now, interface, tx);
        }
        let Some(candidate) = &mut self.candidate else {
            return Ok(());
        };
        if candidate.probe.transmit(now, queue)? {
            let config = candidate.config;
            self.apply(config, "bound", now, interface, tx)?;
        }
        Ok(())
    }

    pub fn transmit(
        &mut self,
        now: u64,
        interface: &mut Interface,
        tx: &mut Outbound,
    ) -> Result<()> {
        let Some(message) = self.pending.as_ref() else {
            return Ok(());
        };
        // Renewal and rebinding cannot originate from a tentative address.
        if message.source != [0; 4] && interface.config().is_none() {
            return Ok(());
        }
        if message.destination == dhcp::LIMITED_BROADCAST {
            let frame = message.broadcast_frame(self.mac).map_err(network_error)?;
            self.pending = None;
            return tx.push(&frame);
        }
        let Some(peer) = interface.peer() else {
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
