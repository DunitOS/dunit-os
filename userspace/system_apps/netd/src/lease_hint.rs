//! Best-effort DHCP address hint on the existing persistent volume.
use dunit::{errno, fs, io::Write, Error, Result};
use dunit_net_core::{Ip, Mac};

use crate::interface::usable_ip;

const PATH: &str = "/persist/netd-dhcp-hint";
const VERSION: u8 = 1;
const SIZE: usize = 11;

pub struct LeaseHint {
    mac: Mac,
    address: Option<Ip>,
}

impl LeaseHint {
    pub fn load(mac: Mac) -> Self {
        let address = fs::read_bounded(PATH, SIZE).ok().and_then(|bytes| {
            let &[version, a, b, c, d, e, f, w, x, y, z] = bytes.as_slice() else {
                return None;
            };
            let address = [w, x, y, z];
            (version == VERSION && [a, b, c, d, e, f] == mac && usable_ip(address))
                .then_some(address)
        });
        Self { mac, address }
    }

    pub fn address(&self) -> Option<Ip> {
        self.address
    }

    pub fn save(&mut self, address: Ip) -> Result<()> {
        if self.address == Some(address) {
            return Ok(());
        }
        self.address = None;
        let [a, b, c, d, e, f] = self.mac;
        let [w, x, y, z] = address;
        // A short record is ignored on startup. Even an intact record can
        // only request revalidation, so losing it during a write is harmless.
        let mut file = fs::File::create(PATH)?;
        file.write_all(&[VERSION, a, b, c, d, e, f, w, x, y, z])?;
        file.sync_all()?;
        file.close()?;
        self.address = Some(address);
        Ok(())
    }

    pub fn clear(&mut self) -> Result<()> {
        self.address = None;
        match fs::remove_file(PATH) {
            Err(Error(errno::ENOENT)) => Ok(()),
            result => result,
        }
    }
}
