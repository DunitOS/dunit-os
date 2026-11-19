//! Address, route and DNS settings are published together with the packet peer.
use dunit_net_core::{
    dhcp::{ClasslessRoutes, CLIENT_PORT},
    Error, Ip, Mac, Peer,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub address: Ip,
    pub mask: Ip,
    pub router: Option<Ip>,
    pub dns: [Option<Ip>; 2],
    pub routes: Option<ClasslessRoutes>,
}

impl Config {
    pub fn static_address(text: &str) -> Option<Self> {
        if text.len() > 512 {
            return None;
        }
        let mut lines = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'));
        let (address, prefix) = lines.next()?.split_once('/')?;
        let prefix = prefix.parse::<u32>().ok()?;
        if !(1..=32).contains(&prefix) {
            return None;
        }
        let mut config = Self {
            address: parse_ip(address)?,
            mask: (u32::MAX << (32 - prefix)).to_be_bytes(),
            router: None,
            dns: [None; 2],
            routes: None,
        };
        for line in lines {
            let (name, value) = line.split_once('=')?;
            match name.trim() {
                "gateway" if config.router.is_none() => {
                    config.router = Some(parse_ip(value.trim())?)
                }
                "dns" if config.dns[0].is_none() => {
                    let mut servers = value.split(',').map(str::trim);
                    config.dns[0] = Some(parse_ip(servers.next()?)?);
                    config.dns[1] = match servers.next() {
                        Some(ip) => Some(parse_ip(ip)?),
                        None => None,
                    };
                    if servers.next().is_some() || config.dns[0] == config.dns[1] {
                        return None;
                    }
                }
                _ => return None,
            }
        }
        Some(config)
    }
}

pub(crate) fn parse_ip(input: &str) -> Option<Ip> {
    let mut address = [0; 4];
    let mut components = input.split('.');
    for octet in &mut address {
        *octet = components.next()?.parse().ok()?;
    }
    if components.next().is_some() || address[0] == 0 || address[0] == 127 || address[0] >= 224 {
        return None;
    }
    Some(address)
}

pub struct Interface {
    mac: Mac,
    mtu: usize,
    active: Option<(Config, Peer)>,
    revision: u64,
}

impl Interface {
    pub fn new(mac: Mac, mtu: usize) -> Self {
        Self {
            mac,
            mtu,
            active: None,
            revision: 0,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn config(&self) -> Option<Config> {
        self.active.as_ref().map(|(config, _)| *config)
    }

    pub fn apply(&mut self, config: Config, dynamic: bool) -> Result<bool, Error> {
        if self.config() == Some(config) {
            return Ok(false);
        }
        let candidate = self.prepare(config, dynamic)?;
        self.active = Some((config, candidate));
        self.revision = self.revision.wrapping_add(1);
        Ok(true)
    }

    pub fn prepare(&self, config: Config, dynamic: bool) -> Result<Peer, Error> {
        let mut candidate = Peer::new(self.mac, config.address, config.mask, self.mtu)?;
        if let Some(routes) = config.routes {
            for route in routes.into_iter().flatten() {
                candidate.add_classless_route(route)?;
            }
        } else if let Some(router) = config.router {
            candidate.add_route([0; 4], [0; 4], router)?;
        }
        if dynamic {
            candidate.udp_bind(CLIENT_PORT)?;
        }
        Ok(candidate)
    }

    pub fn clear(&mut self) -> bool {
        let removed = self.active.take().is_some();
        if removed {
            self.revision = self.revision.wrapping_add(1);
        }
        removed
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
            routes: None,
        };
        assert_eq!(interface.apply(config, true), Ok(true));
        assert_eq!(interface.peer().unwrap().udp_bind(1122), Ok(1122));
        let invalid = Config {
            router: Some([203, 0, 113, 1]),
            ..config
        };
        assert_eq!(interface.apply(invalid, true), Err(Error::Address));
        assert_eq!(interface.config(), Some(config));
        assert_eq!(interface.apply(config, true), Ok(false));
        assert_eq!(
            interface.peer().unwrap().udp_bind(1122),
            Err(Error::PortInUse)
        );
        assert!(interface.clear());
        assert!(interface.peer().is_none());
    }

    #[test]
    fn static_address_preserves_cidr_format_and_prefix_bounds() {
        let config = Config::static_address("192.0.2.2/24\n").unwrap();
        assert_eq!(config.address, [192, 0, 2, 2]);
        assert_eq!(config.mask, [255, 255, 255, 0]);
        assert_eq!(
            Config::static_address("192.0.2.2/32").unwrap().mask,
            [255; 4]
        );
        assert!(Config::static_address("192.0.2.2/0").is_none());
        assert!(Config::static_address("192.0.2.2/33").is_none());
        assert!(Config::static_address("192.0.2.2.3/24").is_none());
    }
    #[test]
    fn static_options_are_bounded_and_validated_before_publication() {
        let text = "# sealed configuration\n192.0.2.12/24\ngateway=192.0.2.1\ndns=192.0.2.53, 192.0.2.54\n";
        let config = Config::static_address(text).unwrap();
        assert_eq!(config.router, Some([192, 0, 2, 1]));
        assert_eq!(config.dns, [Some([192, 0, 2, 53]), Some([192, 0, 2, 54])]);
        let mut interface = Interface::new(MAC, 1500);
        assert!(interface.prepare(config, false).is_ok());
        assert_eq!(interface.revision(), 0);
        assert!(interface.config().is_none());
        assert_eq!(interface.apply(config, false), Ok(true));
        for invalid in [
            "192.0.2.12/24\ngateway=127.0.0.1",
            "192.0.2.12/24\ndns=224.0.0.1",
            "192.0.2.12/24\ndns=192.0.2.53,192.0.2.53",
            "192.0.2.12/24\ndns=192.0.2.1,192.0.2.2,192.0.2.3",
            "192.0.2.12/24\ngateway=192.0.2.1\ngateway=192.0.2.2",
            "192.0.2.12/24\nunknown=yes",
        ] {
            assert!(Config::static_address(invalid).is_none());
        }
        let off_link = Config::static_address("192.0.2.12/24\ngateway=203.0.113.1").unwrap();
        assert_eq!(interface.apply(off_link, false), Err(Error::Address));
        assert_eq!(interface.config(), Some(config));
        assert_eq!(interface.revision(), 1);
    }
}
