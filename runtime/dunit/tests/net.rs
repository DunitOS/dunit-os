use dunit::net::{Ipv4Addr, SocketAddrV4};

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

