use crate::dns::{dual::{DualPoll, DualResolver}, AddressData, Kind, Question};
use crate::ipv6::{self, Ipv6, ALL_NODES_MULTICAST, ALL_ROUTERS_MULTICAST, ETHERTYPE_IPV6, PROTO_ICMPV6, PROTO_TCP, UNSPECIFIED};
use crate::ipv6_peer::{Egress6, Ip6Peer, RxEvent6};
use crate::ndp::Icmpv6;
use crate::router::{InterfaceView, MultiRouter, Route4, Route6};
use crate::tcp::table::{Ingress, Table};
use crate::tcp::{DualEndpoint, State};
use crate::{Ethernet, IpAddr, Mac, MTU};
use alloc::vec::Vec;

const MAC_A: Mac = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
const MAC_R: Mac = [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0x01];
const ROUTER_LL: [u8; 16] = [0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0x00, 0xaa, 0xbb, 0xff, 0xfe, 0xcc, 0xdd, 0x01];
const SERVER_V6: [u8; 16] = [0x20, 0x01, 0x0d, 0xb8, 0, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x50];
const DNS_V6: [u8; 16] = [0x20, 0x01, 0x0d, 0xb8, 0, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x53];

fn encode_ra(
    src: [u8; 16],
    dst: [u8; 16],
    src_mac: Mac,
    prefix: [u8; 16],
    rdnss: [u8; 16],
) -> Vec<u8> {
    let mut ra = Vec::new();
    ra.push(134); // Router Advertisement
    ra.push(0);   // Code
    ra.extend_from_slice(&[0, 0]); // Checksum placeholder
    ra.push(64);  // Cur hop limit
    ra.push(0);   // Flags
    ra.extend_from_slice(&1800u16.to_be_bytes()); // Router lifetime
    ra.extend_from_slice(&0u32.to_be_bytes()); // Reachable time
    ra.extend_from_slice(&0u32.to_be_bytes()); // Retrans timer
    // SLLAO option (kind=1, len=1 -> 8 bytes)
    ra.push(1);
    ra.push(1);
    ra.extend_from_slice(&src_mac);
    // Prefix Info option (kind=3, len=4 -> 32 bytes)
    ra.push(3);
    ra.push(4);
    ra.push(64);
    ra.push(0xc0); // L=1, A=1
    ra.extend_from_slice(&86400u32.to_be_bytes());
    ra.extend_from_slice(&14400u32.to_be_bytes());
    ra.extend_from_slice(&0u32.to_be_bytes());
    ra.extend_from_slice(&prefix);
    // RDNSS option (kind=25, len=3 -> 24 bytes)
    ra.push(25);
    ra.push(3);
    ra.extend_from_slice(&0u16.to_be_bytes());
    ra.extend_from_slice(&3600u32.to_be_bytes());
    ra.extend_from_slice(&rdnss);

    let csum = crate::checksum_pseudo(&src, &dst, PROTO_ICMPV6, &ra);
    ra[2..4].copy_from_slice(&csum.to_be_bytes());
    let pkt = Ipv6::encode(src, dst, PROTO_ICMPV6, 255, &ra).unwrap();
    Ethernet::encode(ALL_NODES_MULTICAST_MAC, src_mac, ETHERTYPE_IPV6, &pkt).unwrap()
}

const ALL_NODES_MULTICAST_MAC: Mac = [0x33, 0x33, 0, 0, 0, 1];

#[test]
fn ipv6_dad_slaac_rdnss_ndp_and_tcp6_flow() {
    let mut peer = Ip6Peer::new(MAC_A, MTU, 1, 0).unwrap();
    let dad_frames = peer.poll(0).unwrap();
    assert_eq!(dad_frames.len(), 1);
    assert!(peer.link_local_ip().is_none());

    // Advance 1s: link-local DAD completes -> RS emitted automatically
    let rs_frames = peer.poll(1_000).unwrap();
    assert_eq!(rs_frames.len(), 1);
    let ll = peer.link_local_ip().unwrap();
    assert!(ipv6::is_link_local(ll));

    let rs_eth = Ethernet::parse(&rs_frames[0]).unwrap();
    assert_eq!(rs_eth.destination, ipv6::multicast_mac(ALL_ROUTERS_MULTICAST));

    // Deliver Router Advertisement with SLAAC /64 + RDNSS
    let prefix = [0x20, 0x01, 0x0d, 0xb8, 0, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let ra_frame = encode_ra(ROUTER_LL, ALL_NODES_MULTICAST, MAC_R, prefix, DNS_V6);
    peer.receive(&ra_frame, 1_100).unwrap();

    // SLAAC address undergoes DAD (1 probe at 1_100, preferred at 2_100)
    assert_eq!(peer.poll(1_100).unwrap().len(), 1);
    assert!(peer.preferred_globals().is_empty());
    peer.poll(2_100).unwrap();
    let globals = peer.preferred_globals();
    assert_eq!(globals.len(), 1);
    let global_ip = globals[0].0;
    assert_eq!(&global_ip[..8], &prefix[..8]);

    let mut view = InterfaceView::empty(0, 10);
    peer.fill_interface_view(&mut view);
    assert_eq!(view.ipv6_dns, alloc::vec![DNS_V6]);
    assert!(!view.ipv6_routes.is_empty());

    // Simulate Router -> Host TCP SYN+ACK & full TCP handshake over IPv6
    let mut client_tcp = Table::new_dual(MTU, 1).unwrap();
    let mut server_tcp = Table::new_dual(MTU, 2).unwrap();
    let srv_lid = server_tcp.listen(443, 2).unwrap();

    let sid = client_tcp
        .connect_ip(
            IpAddr::V6(global_ip),
            DualEndpoint {
                ip: IpAddr::V6(SERVER_V6),
                port: 443,
            },
            2_100,
        )
        .unwrap();

    // Client poll -> SYNs through Ip6Peer -> router already known from RA SLLAO!
    let syn = client_tcp.poll_ip(2_100).unwrap().unwrap();
    let Egress6::Sent(syn_frame) = peer
        .send_packet(
            global_ip,
            SERVER_V6,
            ROUTER_LL,
            PROTO_TCP,
            &syn.bytes,
            2_100,
        )
        .unwrap()
    else {
        panic!("router MAC should already be known from RA SLLAO");
    };
    let eth_syn = Ethernet::parse(&syn_frame).unwrap();
    let ip_syn = Ipv6::parse(eth_syn.payload).unwrap();
    assert_eq!(
        server_tcp
            .receive_ip(
                IpAddr::V6(ip_syn.source),
                IpAddr::V6(ip_syn.destination),
                ip_syn.payload,
                2_100,
            )
            .unwrap(),
        Ingress::Admitted
    );

    // Server SYN-ACK -> back to client
    let synack = server_tcp.poll_ip(2_100).unwrap().unwrap();
    let sa_pkt = Ipv6::encode(SERVER_V6, global_ip, PROTO_TCP, 64, &synack.bytes).unwrap();
    let sa_frame = Ethernet::encode(MAC_A, MAC_R, ETHERTYPE_IPV6, &sa_pkt).unwrap();
    let RxEvent6::Tcp {
        source,
        destination,
        payload,
    } = peer.receive(&sa_frame, 2_110).unwrap()
    else {
        panic!("expected TCP event");
    };
    assert_eq!(
        client_tcp
            .receive_ip(IpAddr::V6(source), IpAddr::V6(destination), payload, 2_110)
            .unwrap(),
        Ingress::Delivered
    );
    assert_eq!(client_tcp.state(sid).unwrap(), State::Established);

    let ack = client_tcp.poll_ip(2_110).unwrap().unwrap();
    assert_eq!(
        server_tcp
            .receive_ip(
                IpAddr::V6(global_ip),
                IpAddr::V6(SERVER_V6),
                &ack.bytes,
                2_120,
            )
            .unwrap(),
        Ingress::Delivered
    );
    let _accepted = server_tcp.accept(srv_lid).unwrap();
}

#[test]
fn dad_detects_address_conflict() {
    let mut peer = Ip6Peer::new(MAC_A, MTU, 2, 0).unwrap();
    peer.poll(0).unwrap();
    let ll = peer.link_local_from_probe_for_test();

    // Another host sends NA for our tentative link-local address
    let na = Icmpv6::encode_na(ll, ALL_NODES_MULTICAST, false, true, ll, MAC_R).unwrap();
    let pkt = Ipv6::encode(ll, ALL_NODES_MULTICAST, PROTO_ICMPV6, 255, &na).unwrap();
    let frame = Ethernet::encode(ALL_NODES_MULTICAST_MAC, MAC_R, ETHERTYPE_IPV6, &pkt).unwrap();
    peer.receive(&frame, 200).unwrap();
    assert!(peer.link_local_failed());
    assert!(peer.link_local_ip().is_none());
}

#[test]
fn multi_router_metrics_and_link_failover() {
    let mut router = MultiRouter::new();
    let mut nic0 = InterfaceView::empty(0, 10);
    nic0.ipv4 = Some(([192, 0, 2, 10], [255, 255, 255, 0]));
    nic0.ipv4_routes.push(Route4 {
        network: [0; 4],
        mask: [0; 4],
        gateway: Some([192, 0, 2, 1]),
    });
    nic0.ipv4_dns.push([192, 0, 2, 53]);
    nic0.ipv6_globals.push((
        [0x20, 0x01, 0x0d, 0xb8, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 10],
        64,
    ));
    nic0.ipv6_routes.push(Route6 {
        prefix: UNSPECIFIED,
        prefix_len: 0,
        gateway: Some(ROUTER_LL),
    });
    nic0.ipv6_dns.push(DNS_V6);

    let mut nic1 = InterfaceView::empty(1, 50);
    nic1.ipv4 = Some(([198, 51, 100, 10], [255, 255, 255, 0]));
    nic1.ipv4_routes.push(Route4 {
        network: [0; 4],
        mask: [0; 4],
        gateway: Some([198, 51, 100, 1]),
    });
    nic1.ipv4_dns.push([198, 51, 100, 53]);

    router.set_interface(nic0.clone());
    router.set_interface(nic1);

    // Lower metric (nic0) wins while up
    assert_eq!(router.select_v4([93, 184, 216, 34]).unwrap().iface_id, 0);
    assert_eq!(router.select_v6(SERVER_V6, 0).unwrap().iface_id, 0);
    assert_eq!(router.dns_servers()[0].server, IpAddr::V6(DNS_V6));

    // Take nic0 down -> IPv4 fails over to nic1 automatically
    nic0.link_up = false;
    router.set_interface(nic0);
    assert_eq!(router.select_v4([93, 184, 216, 34]).unwrap().iface_id, 1);
    assert_eq!(router.dns_servers()[0].server, IpAddr::V4([198, 51, 100, 53]));
}

#[test]
fn dual_dns_resolves_aaaa_and_a_in_one_transaction() {
    let mut resolver = DualResolver::new();
    let servers = [IpAddr::V6(DNS_V6)];
    assert!(resolver
        .start(b"dual.dunit.test", &servers, true, true, 0)
        .unwrap()
        .is_none());

    let DualPoll::SendUdp { server: s1, payload: q1 } = resolver.poll(0).unwrap() else {
        panic!("expected AAAA send");
    };
    let DualPoll::SendUdp { server: s2, payload: q2 } = resolver.poll(0).unwrap() else {
        panic!("expected A send");
    };
    assert_eq!(s1, IpAddr::V6(DNS_V6));
    assert_eq!(s2, IpAddr::V6(DNS_V6));

    let parsed1 = Question::parse(&q1).unwrap();
    let parsed2 = Question::parse(&q2).unwrap();
    assert_eq!(parsed1.kind, Kind::Aaaa);
    assert_eq!(parsed2.kind, Kind::A);

    let r1 = parsed1
        .multi_response(60, &[AddressData::V6(SERVER_V6)])
        .unwrap();
    let r2 = parsed2
        .multi_response(60, &[AddressData::V4([192, 0, 2, 80])])
        .unwrap();
    resolver.receive_udp(IpAddr::V6(DNS_V6), &r1, 10).unwrap();
    resolver.receive_udp(IpAddr::V6(DNS_V6), &r2, 10).unwrap();

    let DualPoll::Resolved(ips) = resolver.poll(10).unwrap() else {
        panic!("expected resolved");
    };
    assert_eq!(
        ips,
        alloc::vec![IpAddr::V6(SERVER_V6), IpAddr::V4([192, 0, 2, 80])]
    );
}

