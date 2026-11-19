//! Route TCP segments through the same IPv4 and ARP path as UDP.
use super::{Endpoint, Segment};
use crate::{Egress, Error, Ethernet, Ipv4, Peer};

impl Peer {
    /// Return a local unicast TCP packet to the connection table. Other
    /// protocols can continue through the usual peer dispatch.
    pub fn tcp_ingress<'a>(&self, bytes: &'a [u8]) -> Result<Option<Ipv4<'a>>, Error> {
        let frame = Ethernet::parse(bytes)?;
        if frame.kind != 0x0800 {
            return Ok(None);
        }
        let packet = Ipv4::parse(frame.payload)?;
        if packet.protocol != 6 {
            return Ok(None);
        }
        if frame.destination != self.mac || packet.destination != self.ip {
            return Err(Error::Address);
        }
        if packet.payload.len() > self.routes.mtu - 20 {
            return Err(Error::Length);
        }
        Ok(Some(packet))
    }

    /// Route a TCP segment, possibly yielding an ARP request first. The caller
    /// owns any segment still waiting for neighbor resolution.
    pub fn tcp_egress(
        &mut self,
        source: Endpoint,
        destination: Endpoint,
        payload: &[u8],
        now: u64,
    ) -> Result<Egress, Error> {
        if source.ip != self.ip {
            return Err(Error::Address);
        }
        Segment::parse(source, destination, payload)?;
        self.route_payload(destination.ip, 6, payload, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Arp, Mac, BROADCAST};
    use crate::tcp::SYN;

    const LOCAL: Endpoint = Endpoint { ip: [192, 0, 2, 2], port: 49152 };
    const REMOTE: Endpoint = Endpoint { ip: [198, 51, 100, 7], port: 80 };
    const GATEWAY: [u8; 4] = [192, 0, 2, 1];
    const MAC: Mac = [2, 0, 0, 0, 0, 2];
    const GATEWAY_MAC: Mac = [2, 0, 0, 0, 0, 1];

    fn syn() -> Segment<'static> {
        Segment { sequence: 19, acknowledgement: 0, flags: SYN, window: 1024, mss: None, data: &[] }
    }

    #[test]
    fn forwards_to_gateway_mac_and_retains_remote_tuple() {
        let mut peer = Peer::new(MAC, LOCAL.ip, [255, 255, 255, 0], 1500).unwrap();
        let segment = syn().encode(LOCAL, REMOTE).unwrap();
        assert_eq!(peer.tcp_egress(LOCAL, REMOTE, &segment, 0), Err(Error::NoRoute));
        peer.add_route([0; 4], [0; 4], GATEWAY).unwrap();
        let Egress::Neighbor(arp) = peer.tcp_egress(LOCAL, REMOTE, &segment, 0).unwrap() else {
            panic!("missing gateway ARP");
        };
        let frame = Ethernet::parse(&arp).unwrap();
        assert_eq!(frame.destination, BROADCAST);
        assert_eq!(Arp::parse(frame.payload).unwrap().target_ip, GATEWAY);
        let reply = Arp {
            operation: 2, sender_mac: GATEWAY_MAC, sender_ip: GATEWAY,
            target_mac: MAC, target_ip: LOCAL.ip,
        }.encode().unwrap();
        peer.receive(&Ethernet::encode(MAC, GATEWAY_MAC, 0x0806, &reply).unwrap(), 1).unwrap();
        let Egress::Sent(frame) = peer.tcp_egress(LOCAL, REMOTE, &segment, 2).unwrap() else {
            panic!("missing routed segment");
        };
        let frame = Ethernet::parse(&frame).unwrap();
        assert_eq!(frame.destination, GATEWAY_MAC);
        let ip = Ipv4::parse(frame.payload).unwrap();
        assert_eq!((ip.source, ip.destination, ip.protocol), (LOCAL.ip, REMOTE.ip, 6));
        assert_eq!(Segment::ports(ip.payload).unwrap(), (LOCAL.port, REMOTE.port));
    }

    #[test]
    fn ingress_requires_local_ethernet_and_ipv4_destinations() {
        let peer = Peer::new(MAC, LOCAL.ip, [255, 255, 255, 0], 1500).unwrap();
        let syn = syn().encode(REMOTE, LOCAL).unwrap();
        let ip = Ipv4::encode_protocol(REMOTE.ip, LOCAL.ip, 6, &syn).unwrap();
        let wrong_mac = Ethernet::encode(BROADCAST, GATEWAY_MAC, 0x0800, &ip).unwrap();
        assert!(matches!(peer.tcp_ingress(&wrong_mac), Err(Error::Address)));
        let right_mac = Ethernet::encode(MAC, GATEWAY_MAC, 0x0800, &ip).unwrap();
        assert_eq!(peer.tcp_ingress(&right_mac).unwrap().unwrap().payload, syn);
        let ip = Ipv4::encode_protocol(REMOTE.ip, GATEWAY, 6, &syn).unwrap();
        let wrong_ip = Ethernet::encode(MAC, GATEWAY_MAC, 0x0800, &ip).unwrap();
        assert!(matches!(peer.tcp_ingress(&wrong_ip), Err(Error::Address)));
    }
}
