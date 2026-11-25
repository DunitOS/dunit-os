//! A boot-configured address waits for a clean probe and withdraws on conflict.
use dunit::Result;
use dunit_net_core::Mac;
use netd::Queue;

use crate::{address_probe::AddressProbe, interface::{Config, Interface}, network_error, outbound::Outbound};

pub struct Static {
    config: Config,
    mac: Mac,
    seed: u32,
    probe: Option<AddressProbe>,
    conflicted: bool,
}

impl Static {
    pub fn new(config: Config, mac: Mac, seed: u32) -> Self {
        Self { config, mac, seed, probe: None, conflicted: false }
    }

    pub fn is_conflicted(&self) -> bool {
        self.conflicted
    }

    pub fn reset(&mut self, up: bool, now: u64, interface: &mut Interface, tx: &mut Outbound) -> Result<()> {
        self.probe = None;
        self.conflicted = false;
        interface.clear();
        tx.clear();
        if up {
            self.seed = self.seed.wrapping_add(0x9e3779b9);
            self.probe = Some(AddressProbe::new(self.mac, self.config.address, self.seed ^ now as u32, now)?);
        }
        Ok(())
    }

    pub fn deadline(&self) -> u64 {
        self.probe.as_ref().and_then(AddressProbe::deadline).unwrap_or(u64::MAX)
    }

    pub fn waiting_for_tx(&self) -> bool {
        self.probe.as_ref().is_some_and(AddressProbe::waiting_for_tx)
    }

    pub fn observe(&mut self, frame: &[u8], interface: &mut Interface, tx: &mut Outbound) -> Result<bool> {
        if !self.probe.as_ref().is_some_and(|probe| probe.observe(frame)) {
            return Ok(false);
        }
        // Keep the interface withdrawn until link renegotiation or reboot.
        self.probe = None;
        self.conflicted = true;
        interface.clear();
        tx.clear();
        let [a, b, c, d] = self.config.address;
        dunit::println!("[NETD] IPv4 static conflict {}.{}.{}.{}", a, b, c, d)?;
        Ok(true)
    }

    pub fn transmit(&mut self, now: u64, queue: &Queue, interface: &mut Interface) -> Result<()> {
        if let Some(probe) = &mut self.probe {
            if probe.transmit(now, queue)? {
                interface.apply(self.config, false).map_err(network_error)?;
                let [a, b, c, d] = self.config.address;
                dunit::println!("[NETD] IPv4 static bound {}.{}.{}.{}", a, b, c, d)?;
            }
        }
        Ok(())
    }
}
