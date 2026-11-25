use dunit::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

#[test]
fn parses_and_formats_ipv4_and_socket_addrs() {
    let ip: Ipv4Addr = "192.0.2.12".parse().unwrap();
    assert_eq!(ip, Ipv4Addr::new(192, 0, 2, 12));
    assert_eq!(ip.to_string(), "192.0.2.12");

    let addr: SocketAddrV4 = "192.0.2.1:5432".parse().unwrap();
    assert_eq!(addr, SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 1), 5432));
    assert_eq!(addr.to_string(), "192.0.2.1:5432");

    for bad in ["", "1.2.3", "1.2.3.4.5", "256.0.0.1", "1.2.3.+4", "1.2.3.4"] {
        assert!(bad.parse::<SocketAddrV4>().is_err(), "{bad}");
    }
    for bad in ["1.2.3.4:", ":80", "1.2.3.4:65536", "1.2.3.4:-1"] {
        assert!(bad.parse::<SocketAddrV4>().is_err(), "{bad}");
    }
}

#[test]
fn parses_and_formats_ipv6_and_dual_socket_addrs() {
    let ip6: Ipv6Addr = "2001:db8:1::10".parse().unwrap();
    assert_eq!(ip6, Ipv6Addr::new(0x2001, 0x0db8, 1, 0, 0, 0, 0, 0x0010));
    let s6: SocketAddr = "[2001:db8:1::10]:443".parse().unwrap();
    assert_eq!(s6, SocketAddr::V6(SocketAddrV6::new(ip6, 443, 0)));
    let scoped: SocketAddr = "[fe80::1%2]:8080".parse().unwrap();
    assert_eq!(scoped.port(), 8080);
    let ip: IpAddr = "2001:db8::1".parse().unwrap();
    assert!(matches!(ip, IpAddr::V6(_)));
}

