//! Apply an address-probe step only when the NIC accepts its packet record.
use dunit::{errno, Error, Result};
use dunit_net_core::{acd::{Action, Probe}, Ip, Mac};
use dunit_packet_abi::encode_record;
use netd::{wire_error, Queue};

use crate::network_error;

pub struct AddressProbe {
    probe: Probe,
    waiting_for_tx: bool,
}

impl AddressProbe {
    pub fn new(mac: Mac, address: Ip, seed: u32, now: u64) -> Result<Self> {
        Ok(Self {
            probe: Probe::new(mac, address, seed, now).map_err(network_error)?,
            waiting_for_tx: false,
        })
    }

    pub fn deadline(&self) -> Option<u64> {
        if self.waiting_for_tx { None } else { self.probe.deadline() }
    }

    pub fn waiting_for_tx(&self) -> bool {
        self.waiting_for_tx
    }

    pub fn observe(&self, frame: &[u8]) -> bool {
        self.probe.observe(frame)
    }

    /// True when the first announcement was accepted and the interface may
    /// claim its address. A blocked write leaves the timer and phase intact.
    pub fn transmit(&mut self, now: u64, queue: &Queue) -> Result<bool> {
        let mut next = self.probe;
        let (frame, claim) = match next.poll(now).map_err(network_error)? {
            Action::Idle => return Ok(false),
            Action::Probe(frame) | Action::Announce(frame) => (frame, false),
            Action::Claim(frame) => (frame, true),
        };
        // Native record header plus a minimum Ethernet frame, rounded to 8.
        let mut record = [0; 72];
        let size = encode_record(&frame, &mut record).map_err(wire_error)?;
        match queue.try_send(&record[..size]) {
            Ok(written) if written == size => {
                self.probe = next;
                self.waiting_for_tx = false;
                Ok(claim)
            }
            Ok(_) => Err(Error(errno::EIO)),
            Err(Error(errno::EAGAIN)) => {
                self.waiting_for_tx = true;
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }
}
