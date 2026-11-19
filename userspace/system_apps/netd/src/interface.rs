//! Address, route and DNS settings are published together with the packet peer.
use dunit_net_core::{dhcp::CLIENT_PORT, Error, Ip, Mac, Peer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub address: Ip,
    pub mask: Ip,
    pub router: Option<Ip>,
    pub dns: [Option<Ip>; 2],
}

impl Config {
    pub fn static_address(text: &str) -> Option<Self> {
        let (address, prefix) = text.trim().split_once('/')?;
        let mut address_bytes = [0; 4];
        let mut components = address.split('.');
        for octet in &mut address_bytes {
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
        Some(Self {
            address: address_bytes,
            mask: mask.to_be_bytes(),
            router: None,
            dns: [None; 2],
        })
    }
}

pub struct Interface {
    mac: Mac,
    mtu: usize,
    active: Option<(Config, Peer)>,
}

impl Interface {
    pub fn new(mac: Mac, mtu: usize) -> Self {
        Self { mac, mtu, active: None }
    }

    pub fn config(&self) -> Option<Config> {
        self.active.as_ref().map(|(config, _)| *config)
    }

    pub fn apply(&mut self, config: Config, dynamic: bool) -> Result<bool, Error> {
        if self.config() == Some(config) {
            return Ok(false);
        }
        let mut candidate = Peer::new(self.mac, config.address, config.mask, self.mtu)?;
        if let Some(router) = config.router {
            candidate.add_route([0; 4], [0; 4], router)?;
        }
        if dynamic {
            candidate.udp_bind(CLIENT_PORT)?;
        }
        self.active = Some((config, candidate));
        Ok(true)
    }

    pub fn clear(&mut self) -> bool {
        self.active.take().is_some()
    }

    pub fn peer(&mut self) -> Option<&mut Peer> {
        self.active.as_mut().map(|(_, peer)| peer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MAC: Mac = [2, 0, 0, 0, 0, 1];

    #[test]
    fn invalid_route_retains_working_config_and_identical_renewal_retains_peer() {
        let mut interface = Interface::new(MAC, 1500);
        let config = Config {
            address: [192, 0, 2, 2],
            mask: [255, 255, 255, 0],
            router: Some([192, 0, 2, 1]),
            dns: [Some([192, 0, 2, 53]), None],
        };
        assert_eq!(interface.apply(config, true), Ok(true));
        assert_eq!(interface.peer().unwrap().udp_bind(1122), Ok(1122));
        let invalid = Config { router: Some([203, 0, 113, 1]), ..config };
        assert_eq!(interface.apply(invalid, true), Err(Error::Address));
        assert_eq!(interface.config(), Some(config));
        assert_eq!(interface.apply(config, true), Ok(false));
        assert_eq!(interface.peer().unwrap().udp_bind(1122), Err(Error::PortInUse));
        assert!(interface.clear());
        assert!(interface.peer().is_none());
    }

    #[test]
    fn static_address_preserves_cidr_format_and_prefix_bounds() {
        let config = Config::static_address("192.0.2.2/24\n").unwrap();
        assert_eq!(config.address, [192, 0, 2, 2]);
        assert_eq!(config.mask, [255, 255, 255, 0]);
        assert_eq!(Config::static_address("192.0.2.2/32").unwrap().mask, [255; 4]);
        assert!(Config::static_address("192.0.2.2/0").is_none());
        assert!(Config::static_address("192.0.2.2/33").is_none());
        assert!(Config::static_address("192.0.2.2.3/24").is_none());
    }
}
