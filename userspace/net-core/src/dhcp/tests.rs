use super::*;
use crate::word;

const CLIENT: Mac = [2, 0, 0, 0, 0, 3];
const SERVER_MAC: Mac = [2, 0, 0, 0, 0, 2];
const SERVER: Ip = [192, 0, 2, 1];
const OTHER: Ip = [192, 0, 2, 4];
const HOST: Ip = [192, 0, 2, 7];
const MASK: Ip = [255, 255, 255, 0];
const DNS: Ip = [198, 51, 100, 53];
const LIFETIME: u32 = 16;

fn offer_options(lifetime: u32) -> Vec<u8> {
    let mut options = alloc::vec![
        1, 4, MASK[0], MASK[1], MASK[2], MASK[3], 3, 4, SERVER[0], SERVER[1], SERVER[2], SERVER[3],
        6, 8, DNS[0], DNS[1], DNS[2], DNS[3], OTHER[0], OTHER[1], OTHER[2], OTHER[3], 51, 4,
    ];
    options.extend_from_slice(&lifetime.to_be_bytes());
    options
}

fn reply(kind: u8, xid: u32, address: Ip, server: Ip, options: &[u8]) -> Vec<u8> {
    let mut payload = alloc::vec![0; 240];
    payload[..4].copy_from_slice(&[2, 1, 6, 0]);
    payload[4..8].copy_from_slice(&xid.to_be_bytes());
    payload[16..20].copy_from_slice(&address);
    payload[28..34].copy_from_slice(&CLIENT);
    payload[236..240].copy_from_slice(&[99, 130, 83, 99]);
    payload.extend_from_slice(&[53, 1, kind, 54, 4]);
    payload.extend_from_slice(&server);
    payload.extend_from_slice(options);
    payload.push(255);
    payload
}

fn server_frame(payload: &[u8], server: Ip, ip: Ip, mac: Mac) -> Vec<u8> {
    frame(
        SERVER_MAC,
        mac,
        Address {
            ip: server,
            port: SERVER_PORT,
        },
        Address {
            ip,
            port: CLIENT_PORT,
        },
        payload,
    )
    .unwrap()
}

fn send(client: &mut Client, now: u64) -> Transmission {
    let Action::Send(output) = client.poll(now).unwrap() else {
        panic!("expected send in {:?} at {now}", client.state());
    };
    output
}

fn transaction(transmission: &Transmission) -> u32 {
    u32::from_be_bytes(transmission.payload[4..8].try_into().unwrap())
}

fn request_header(transmission: &Transmission, kind: u8, ip: Ip, server: Ip) {
    assert_eq!(transmission.source, ip);
    assert_eq!(transmission.destination, server);
    assert_eq!(&transmission.payload[..4], &[1, 1, 6, 0]);
    assert_eq!(transmission.payload[12..16], ip);
    assert_eq!(transmission.payload[28..34], CLIENT);
    assert_eq!(transmission.payload[240..243], [53, 1, kind]);
    assert_ne!(transaction(transmission), 0);
}

fn receive(
    client: &mut Client,
    now: u64,
    kind: u8,
    xid: u32,
    address: Ip,
    server: Ip,
) -> Result<Event, Error> {
    let payload = reply(kind, xid, address, server, &offer_options(LIFETIME));
    client.receive(
        &server_frame(&payload, server, LIMITED_BROADCAST, BROADCAST),
        now,
    )
}

fn select(client: &mut Client) -> u32 {
    let xid = transaction(&send(client, 0));
    assert_eq!(receive(client, 1, 2, xid, HOST, SERVER), Ok(Event::Offered));
    xid
}

fn bound(client: &mut Client) -> Lease {
    let xid = select(client);
    send(client, 1);
    let Event::Bound(lease) = receive(client, 2, 5, xid, HOST, SERVER).unwrap() else {
        panic!("expected initial lease");
    };
    lease
}

#[test]
fn remembered_address_waits_for_matching_ack_with_fresh_configuration() {
    let mut client = Client::new(CLIENT, 71).unwrap();
    assert_eq!(client.revalidate(UNASSIGNED, 0), Err(Error::Address));
    client.revalidate(HOST, 0).unwrap();
    let request = send(&mut client, 0);
    request_header(&request, 3, UNASSIGNED, LIMITED_BROADCAST);
    assert_eq!(word(&request.payload, 10), 0x8000);
    assert_eq!(&request.payload[243..249], &[50, 4, 192, 0, 2, 7]);
    assert_eq!(request.payload[249], 61); // Client identifier follows, with no server ID.
    let xid = transaction(&request);
    assert_eq!(client.lease(), None);
    assert_eq!(
        receive(&mut client, 1, 5, xid ^ 1, HOST, SERVER),
        Err(Error::Unsolicited)
    );
    assert_eq!(
        receive(&mut client, 1, 5, xid, OTHER, SERVER),
        Err(Error::Unsolicited)
    );
    let no_mask = reply(5, xid, HOST, OTHER, &[51, 4, 0, 0, 0, 16]);
    assert_eq!(
        client.receive(&server_frame(&no_mask, OTHER, HOST, CLIENT), 1),
        Err(Error::Header)
    );
    assert_eq!(client.lease(), None);
    let Ok(Event::Bound(lease)) = receive(&mut client, 2, 5, xid, HOST, OTHER) else {
        panic!("expected fresh configuration for remembered address");
    };
    assert_eq!(lease.address, HOST);
    assert_eq!(lease.server, OTHER);
    assert_eq!(lease.expires_at, 16_002);
}

#[test]
fn remembered_address_is_discarded_on_nak_or_retry_exhaustion() {
    for nak in [true, false] {
        let mut client = Client::new(CLIENT, 73).unwrap();
        client.revalidate(HOST, 0).unwrap();
        let first = send(&mut client, 0);
        let xid = transaction(&first);
        if nak {
            assert_eq!(receive(&mut client, 1, 6, xid, UNASSIGNED, SERVER), Ok(Event::Lost));
        } else {
            for _ in 1..REQUEST_ATTEMPTS {
                let now = client.deadline();
                let request = send(&mut client, now);
                request_header(&request, 3, UNASSIGNED, LIMITED_BROADCAST);
                assert_eq!(transaction(&request), xid);
            }
        }
        assert_eq!(client.lease(), None);
        let now = client.deadline();
        let discover = send(&mut client, now);
        request_header(&discover, 1, UNASSIGNED, LIMITED_BROADCAST);
        assert_ne!(transaction(&discover), xid);
        assert_eq!(
            receive(&mut client, now, 5, xid, HOST, SERVER),
            Err(Error::Unsolicited)
        );
    }
}

#[test]
fn direct_dora_uses_broadcast_exceptions_and_selects_exact_server() {
    let mut client = Client::new(CLIENT, 95).unwrap();
    let discover = send(&mut client, 0);
    request_header(&discover, 1, UNASSIGNED, LIMITED_BROADCAST);
    assert_eq!(word(&discover.payload, 10), 0x8000);
    assert!(discover
        .payload
        .windows(6)
        .any(|option| option == [55, 4, 1, 121, 3, 6]));
    let wire = discover.broadcast_frame(CLIENT).unwrap();
    let ethernet = Ethernet::parse(&wire).unwrap();
    assert_eq!(ethernet.destination, BROADCAST);
    assert!(matches!(Ipv4::parse(ethernet.payload), Err(Error::Address)));
    let ip = Ipv4::parse_framing(ethernet.payload).unwrap();
    let udp = Udp::parse_framing(ip.source, ip.destination, ip.payload).unwrap();
    assert_eq!((udp.source, udp.destination), (CLIENT_PORT, SERVER_PORT));
    assert_eq!(udp.data, discover.payload);
    assert!(!is_candidate(&wire));

    let xid = transaction(&discover);
    assert_eq!(
        receive(&mut client, 1, 2, xid, HOST, SERVER),
        Ok(Event::Offered)
    );
    assert_eq!(client.state(), State::Requesting);
    assert_eq!(client.deadline(), 1);
    assert_eq!(
        receive(&mut client, 1, 5, xid, HOST, SERVER),
        Err(Error::Unsolicited)
    );
    assert_eq!(client.lease(), None);
    let request = send(&mut client, 1);
    request_header(&request, 3, UNASSIGNED, LIMITED_BROADCAST);
    assert_eq!(transaction(&request), xid);
    assert_eq!(
        &request.payload[243..255],
        &[50, 4, 192, 0, 2, 7, 54, 4, 192, 0, 2, 1]
    );
    assert_eq!(
        receive(&mut client, 2, 2, xid, HOST, SERVER),
        Ok(Event::Ignored)
    );
    assert_eq!(
        receive(&mut client, 2, 5, xid, HOST, OTHER),
        Err(Error::Unsolicited)
    );
    assert_eq!(
        receive(&mut client, 2, 5, xid, OTHER, SERVER),
        Err(Error::Unsolicited)
    );

    let payload = reply(5, xid, HOST, SERVER, &offer_options(LIFETIME));
    let unicast_ack = server_frame(&payload, SERVER, HOST, CLIENT);
    assert!(is_candidate(&unicast_ack));
    let Event::Bound(lease) = client.receive(&unicast_ack, 3).unwrap() else {
        panic!("expected accepted ACK");
    };
    assert_eq!(lease.address, HOST);
    assert_eq!(lease.mask, MASK);
    assert_eq!(lease.router, Some(SERVER));
    assert_eq!(lease.dns, [Some(DNS), Some(OTHER)]);
    assert_eq!(
        (lease.renew_at, lease.rebind_at, lease.expires_at),
        (8003, 14003, 16003)
    );
    assert_eq!(client.receive(&unicast_ack, 4), Ok(Event::Ignored));
    assert_eq!(client.poll(lease.renew_at - 1), Ok(Action::Idle));
}

#[test]
fn renewal_and_rebinding_follow_lease_deadlines_and_accept_new_server() {
    let mut client = Client::new(CLIENT, 1).unwrap();
    let lease = bound(&mut client);
    let renewal = send(&mut client, lease.renew_at);
    request_header(&renewal, 3, HOST, SERVER);
    assert_eq!(
        renewal.payload[243], 61,
        "select options must be absent in renew"
    );
    assert_eq!(client.state(), State::Renewing);
    let xid = transaction(&renewal);
    assert_eq!(
        receive(&mut client, lease.renew_at + 1, 5, xid, HOST, OTHER),
        Err(Error::Unsolicited)
    );

    let rebind = send(&mut client, lease.rebind_at);
    request_header(&rebind, 3, HOST, LIMITED_BROADCAST);
    assert_eq!(transaction(&rebind), xid);
    assert_eq!(client.state(), State::Rebinding);
    let reply = reply(
        5,
        xid,
        HOST,
        OTHER,
        &[51, 4, 0, 0, 0, 10, 58, 4, 0, 0, 0, 2, 59, 4, 0, 0, 0, 8],
    );
    let ack = server_frame(&reply, OTHER, HOST, CLIENT);
    let now = lease.rebind_at + 100;
    let Event::Bound(extended) = client.receive(&ack, now).unwrap() else {
        panic!("expected extended lease");
    };
    assert_eq!(extended.server, OTHER);
    assert_eq!(extended.mask, lease.mask);
    assert_eq!(extended.router, lease.router);
    assert_eq!(extended.dns, lease.dns);
    assert_eq!(
        (extended.renew_at, extended.rebind_at, extended.expires_at),
        (now + 2000, now + 8000, now + 10000)
    );
    let future = send(&mut client, extended.renew_at);
    request_header(&future, 3, HOST, OTHER);
    assert_ne!(transaction(&future), xid);
}

#[test]
fn expiry_and_link_reset_withdraw_configuration_before_reuse() {
    let mut client = Client::new(CLIENT, 2).unwrap();
    let lease = bound(&mut client);
    let request = send(&mut client, lease.rebind_at);
    assert_eq!(
        client.state(),
        State::Rebinding,
        "late timer can enter rebind directly"
    );
    let old = transaction(&request);
    assert_eq!(client.deadline(), lease.expires_at - 1000);
    assert_eq!(client.poll(lease.expires_at), Ok(Action::Expired));
    assert_eq!(client.lease(), None);
    let next = send(&mut client, lease.expires_at);
    assert_ne!(transaction(&next), old);
    assert_eq!(client.poll(lease.expires_at - 1), Err(Error::Header));
    client.reset(lease.expires_at + 10);
    assert_eq!(client.state(), State::Init);
    assert_eq!(client.deadline(), lease.expires_at + 10);

    let mut other = Client::new(CLIENT, 2).unwrap();
    let lease = bound(&mut other);
    let receive_after_expiry = other.receive(&[], lease.expires_at);
    assert_eq!(receive_after_expiry, Ok(Event::Lost));
    assert_eq!(other.lease(), None);
}

#[test]
fn selected_server_nak_restarts_with_new_transaction_and_why_ack_is_ignored() {
    let mut client = Client::new(CLIENT, 5).unwrap();
    let lease = bound(&mut client);
    let xid = transaction(&send(&mut client, lease.renew_at));
    let now = lease.renew_at + 10;
    let foreign = reply(6, xid, UNASSIGNED, OTHER, &[]);
    assert_eq!(
        client.receive(&server_frame(&foreign, OTHER, HOST, CLIENT), now),
        Err(Error::Unsolicited)
    );
    assert_eq!(client.lease(), Some(lease));
    let invalid_nak = reply(6, xid, HOST, SERVER, &[]);
    assert_eq!(
        client.receive(&server_frame(&invalid_nak, SERVER, HOST, CLIENT), now),
        Err(Error::Header)
    );
    let nak = reply(6, xid, UNASSIGNED, SERVER, &[]);
    assert_eq!(
        client.receive(
            &server_frame(&nak, SERVER, LIMITED_BROADCAST, BROADCAST),
            now
        ),
        Ok(Event::Lost)
    );
    assert_eq!(client.lease(), None);
    assert_eq!(client.deadline(), now + 1000);
    assert_eq!(client.poll(now + 999), Ok(Action::Idle));
    let restarted = send(&mut client, now + 1000);
    assert_ne!(transaction(&restarted), xid);
    assert_eq!(
        client.receive(
            &server_frame(&nak, SERVER, LIMITED_BROADCAST, BROADCAST),
            now + 1001
        ),
        Err(Error::Unsolicited)
    );
}

#[test]
fn retries_are_bounded_and_roll_xid_after_unanswered_discovery_or_request() {
    let mut client = Client::new(CLIENT, 42).unwrap();
    let xid = transaction(&send(&mut client, 1));
    for attempt in 0..3 {
        let previous = client.clock;
        let next = client.deadline();
        let base = 3500 << attempt;
        assert!((base..=base + 1023).contains(&(next - previous)));
        assert_eq!(client.poll(next - 1), Ok(Action::Idle));
        assert_eq!(transaction(&send(&mut client, next)), xid);
    }
    let next = client.deadline();
    let fresh = transaction(&send(&mut client, next));
    assert_ne!(fresh, xid);
    assert_eq!(
        receive(&mut client, next + 1, 2, fresh, HOST, SERVER),
        Ok(Event::Offered)
    );
    for _ in 0..REQUEST_ATTEMPTS {
        let next = client.deadline();
        let result = send(&mut client, next);
        request_header(&result, 3, UNASSIGNED, LIMITED_BROADCAST);
        assert_eq!(transaction(&result), fresh);
    }
    let next = client.deadline();
    let result = send(&mut client, next);
    request_header(&result, 1, UNASSIGNED, LIMITED_BROADCAST);
    assert_ne!(transaction(&result), fresh);
}

#[test]
fn malformed_options_do_not_publish_an_offer_or_replace_selected_config() {
    let mut client = Client::new(CLIENT, 9).unwrap();
    let xid = transaction(&send(&mut client, 0));
    for (tail, error) in [
        (alloc::vec![51, 4, 0, 0, 0, 1], Error::Header),
        (alloc::vec![1, 4, 255, 0, 255, 0], Error::Header),
        (alloc::vec![53, 1, 2], Error::Header),
        (alloc::vec![54, 4, 192, 0, 2, 1], Error::Header),
        (alloc::vec![52, 1, 1], Error::Unsupported),
        (
            alloc::vec![58, 4, 0, 0, 0, 9, 59, 4, 0, 0, 0, 8],
            Error::Header,
        ),
        (alloc::vec![255, 1], Error::Header),
        (alloc::vec![199, 255], Error::Length),
    ] {
        let mut options = offer_options(LIFETIME);
        options.extend_from_slice(&tail);
        let malformed = reply(2, xid, HOST, SERVER, &options);
        let wire = server_frame(&malformed, SERVER, LIMITED_BROADCAST, BROADCAST);
        assert_eq!(client.receive(&wire, 1), Err(error), "tail {tail:?}");
        assert_eq!(client.state(), State::Selecting);
        assert_eq!(client.lease(), None);
    }

    let unreachable_router = [
        1, 4, 255, 255, 255, 0, 51, 4, 0, 0, 0, 16, 3, 4, 10, 0, 0, 1,
    ];
    let malformed = reply(2, xid, HOST, SERVER, &unreachable_router);
    assert_eq!(
        client.receive(&server_frame(&malformed, SERVER, HOST, CLIENT), 1),
        Err(Error::Address)
    );
    assert_eq!(
        receive(&mut client, 1, 2, xid, HOST, SERVER),
        Ok(Event::Offered)
    );
    send(&mut client, 1);
    let bad_timer = [
        51, 4, 0, 0, 0, 16, 1, 4, 255, 255, 255, 0, 58, 4, 0, 0, 0, 0,
    ];
    let bad_ack = reply(5, xid, HOST, SERVER, &bad_timer);
    assert_eq!(
        client.receive(&server_frame(&bad_ack, SERVER, HOST, CLIENT), 2),
        Err(Error::Header)
    );
    assert_eq!(client.state(), State::Requesting);
    assert!(matches!(
        receive(&mut client, 2, 5, xid, HOST, SERVER),
        Ok(Event::Bound(_))
    ));
}

#[test]
fn validates_udp_checksums_xid_and_direct_link_identity() {
    let mut client = Client::new(CLIENT, 11).unwrap();
    let xid = transaction(&send(&mut client, 0));
    let reply = reply(2, xid, HOST, SERVER, &offer_options(LIFETIME));
    let legitimate = server_frame(&reply, SERVER, HOST, CLIENT);
    let mut corrupt = legitimate.clone();
    let last = corrupt.len() - 1;
    corrupt[last] ^= 1;
    assert_eq!(client.receive(&corrupt, 1), Err(Error::Checksum));

    let mut forged = reply.clone();
    forged[4] ^= 1;
    assert_eq!(
        client.receive(&server_frame(&forged, SERVER, HOST, CLIENT), 1),
        Err(Error::Unsolicited)
    );
    forged[4] ^= 1;
    forged[28] ^= 1;
    assert_eq!(
        client.receive(&server_frame(&forged, SERVER, HOST, CLIENT), 1),
        Err(Error::Unsolicited)
    );
    assert_eq!(
        client.receive(&server_frame(&reply, OTHER, HOST, CLIENT), 1),
        Err(Error::Unsolicited)
    );
    assert_eq!(
        client.receive(&server_frame(&reply, SERVER, HOST, SERVER_MAC), 1),
        Err(Error::Unsolicited)
    );

    assert_eq!(client.receive(&legitimate, 1), Ok(Event::Offered));
}

#[test]
fn relayed_reply_keeps_server_identity_for_selection_and_renewal() {
    let remote_server = [198, 51, 100, 53];
    let mut client = Client::new(CLIENT, 41).unwrap();
    let xid = transaction(&send(&mut client, 0));
    let config = offer_options(LIFETIME);
    let mut offer = reply(2, xid, HOST, remote_server, &config);
    offer[3] = 1;
    offer[24..28].copy_from_slice(&SERVER);
    let broadcast = |payload: &[u8], source| {
        server_frame(payload, source, LIMITED_BROADCAST, BROADCAST)
    };
    assert_eq!(
        client.receive(&broadcast(&offer, OTHER), 1),
        Err(Error::Unsolicited)
    );
    let mut invalid = offer.clone();
    invalid[3] = 17;
    assert_eq!(client.receive(&broadcast(&invalid, SERVER), 1), Err(Error::Header));
    invalid[3] = 1;
    invalid[24..28].copy_from_slice(&UNASSIGNED);
    assert_eq!(client.receive(&broadcast(&invalid, SERVER), 1), Err(Error::Header));
    invalid[24..28].copy_from_slice(&LIMITED_BROADCAST);
    assert_eq!(client.receive(&broadcast(&invalid, SERVER), 1), Err(Error::Address));
    assert_eq!(client.state(), State::Selecting);
    assert_eq!(client.receive(&broadcast(&offer, SERVER), 1), Ok(Event::Offered));
    let request = send(&mut client, 1);
    assert!(request.payload.windows(6).any(|option| option == [54, 4, 198, 51, 100, 53]));

    let mut ack = reply(5, xid, HOST, OTHER, &config);
    ack[24..28].copy_from_slice(&SERVER);
    assert_eq!(client.receive(&broadcast(&ack, SERVER), 2), Err(Error::Unsolicited));
    let mut ack = reply(5, xid, HOST, remote_server, &config);
    ack[24..28].copy_from_slice(&SERVER);
    let direct_source = server_frame(&ack, remote_server, HOST, CLIENT);
    let Ok(Event::Bound(lease)) = client.receive(&direct_source, 2) else {
        panic!("expected relayed lease");
    };
    assert_eq!(lease.server, remote_server);
    let renewal = send(&mut client, lease.renew_at);
    request_header(&renewal, 3, HOST, remote_server);
}

#[test]
fn option_bounds_and_mutations_never_panic() {
    let mut payload = reply(2, 9, HOST, SERVER, &offer_options(LIFETIME));
    assert!(wire::reply(&payload).is_ok());
    for length in 0..payload.len() {
        assert!(wire::reply(&payload[..length]).is_err());
    }

    for index in 0..payload.len() {
        let saved = payload[index];
        for bits in [0, 0x55, 0xff] {
            payload[index] = bits;
            let _ = wire::reply(&payload);
        }
        payload[index] = saved;
    }
    let options = [231, 0].repeat(128);
    let excessive = reply(2, 9, HOST, SERVER, &options);
    assert!(matches!(wire::reply(&excessive), Err(Error::Capacity)));
    payload.resize(crate::udp::MAX_PAYLOAD + 1, 0);
    assert!(matches!(wire::reply(&payload), Err(Error::Length)));
}

#[test]
fn infinite_lease_never_schedules_renewal_and_invalid_mac_is_refused() {
    assert!(matches!(Client::new([0; 6], 1), Err(Error::Address)));
    let mut client = Client::new(CLIENT, 0).unwrap();
    let xid = select(&mut client);
    send(&mut client, 1);
    let payload = reply(5, xid, HOST, SERVER, &offer_options(u32::MAX));
    let frame = server_frame(&payload, SERVER, HOST, CLIENT);
    let Event::Bound(lease) = client.receive(&frame, 2).unwrap() else {
        panic!("infinite lease refused");
    };
    assert_eq!(lease.expires_at, u64::MAX);
    assert_eq!(client.deadline(), u64::MAX);
    assert_eq!(client.poll(u64::MAX - 1000), Ok(Action::Idle));
}

#[test]
fn decline_withdraws_acknowledged_lease_and_cools_down_across_link_resets() {
    let mut client = Client::new(CLIENT, 92).unwrap();
    assert_eq!(client.decline(0), Err(Error::NotBound));
    let xid = select(&mut client);
    assert_eq!(client.decline(1), Err(Error::NotBound));
    send(&mut client, 1);
    assert_eq!(client.decline(1), Err(Error::NotBound));
    let Event::Bound(lease) = receive(&mut client, 2, 5, xid, HOST, SERVER).unwrap() else {
        panic!("expected validated lease");
    };

    // A renewal started before the conflict uses a separate xid. Decline
    // identifies the lease whose ACK was already accepted.
    let renewal = send(&mut client, lease.renew_at);
    assert_ne!(transaction(&renewal), xid);
    assert_eq!(client.decline(lease.renew_at - 1), Err(Error::Header));
    assert_eq!(client.lease(), Some(lease));
    let now = lease.renew_at + 1;
    let declined = client.decline(now).unwrap();
    request_header(&declined, 4, UNASSIGNED, LIMITED_BROADCAST);
    assert_eq!(transaction(&declined), xid);
    assert_eq!(&declined.payload[8..28], &[0; 20]);
    assert_eq!(
        &declined.payload[243..265],
        &[50, 4, 192, 0, 2, 7, 54, 4, 192, 0, 2, 1, 61, 7, 1, 2, 0, 0, 0, 0, 3, 255,]
    );
    assert!(declined.payload[265..].iter().all(|byte| *byte == 0));

    let wire = declined.broadcast_frame(CLIENT).unwrap();
    let ethernet = Ethernet::parse(&wire).unwrap();
    assert_eq!(ethernet.destination, BROADCAST);
    let ip = Ipv4::parse_framing(ethernet.payload).unwrap();
    assert_eq!((ip.source, ip.destination), (UNASSIGNED, LIMITED_BROADCAST));
    let udp = Udp::parse_framing(ip.source, ip.destination, ip.payload).unwrap();
    assert_eq!((udp.source, udp.destination), (CLIENT_PORT, SERVER_PORT));
    assert_eq!(udp.data, declined.payload);

    assert_eq!(client.state(), State::Init);
    assert_eq!(client.lease(), None);
    assert_eq!(client.decline(now), Err(Error::NotBound));
    client.reset(now + 1);
    assert_eq!(client.deadline(), now + DECLINE_RETRY_MS);
    assert_eq!(client.poll(now + DECLINE_RETRY_MS - 1), Ok(Action::Idle));
    let rediscovery = send(&mut client, now + DECLINE_RETRY_MS);
    request_header(&rediscovery, 1, UNASSIGNED, LIMITED_BROADCAST);
    assert_ne!(transaction(&rediscovery), xid);
}

#[test]
fn expired_lease_cannot_send_decline() {
    let mut client = Client::new(CLIENT, 15).unwrap();
    let lease = bound(&mut client);
    assert_eq!(client.decline(lease.expires_at), Err(Error::NotBound));
    assert_eq!(client.poll(lease.expires_at), Ok(Action::Expired));
}

fn with_classless(entries: &[u8]) -> Vec<u8> {
    let mut options = offer_options(LIFETIME);
    options.extend_from_slice(&[121, entries.len() as u8]);
    options.extend_from_slice(entries);
    options
}

#[test]
fn classless_routes_override_router_and_select_longest_prefix() {
    let mut client = Client::new(CLIENT, 29).unwrap();
    let xid = select(&mut client);
    send(&mut client, 1);

    // Fragments split the default entry itself. Router's off-link value would
    // be invalid were a classless option absent.
    let mut options = [
        1, 4, 255, 255, 255, 0, 51, 4, 0, 0, 0, 16, 3, 4, 10, 0, 0, 1, 121, 2, 0, 192,
    ]
    .to_vec();
    options.extend_from_slice(&[
        121, 21, 0, 2, 4, 25, 198, 51, 100, 128, 192, 0, 2, 1, 32, 198, 51, 100, 200, 0, 0, 0, 0,
    ]);
    let ack = reply(5, xid, HOST, SERVER, &options);
    let Event::Bound(lease) = client
        .receive(&server_frame(&ack, SERVER, HOST, CLIENT), 2)
        .unwrap()
    else {
        panic!("expected accepted classless ACK");
    };
    assert_eq!(lease.router, None);
    let advertised = lease.routes.unwrap();
    assert_eq!(advertised.iter().flatten().count(), 3);
    let mut routes = Routes::new(lease.address, lease.mask, MTU).unwrap();
    for route in advertised.iter().flatten() {
        route.install(&mut routes).unwrap();
    }
    assert_eq!(routes.next_hop([198, 51, 100, 120]), Ok(OTHER));
    assert_eq!(routes.next_hop([198, 51, 100, 132]), Ok(SERVER));
    assert_eq!(routes.next_hop([198, 51, 100, 200]), Ok([198, 51, 100, 200]));
}

#[test]
fn classless_routes_survive_same_server_ack_and_are_replaced_on_rebind() {
    let mut client = Client::new(CLIENT, 31).unwrap();
    let xid = select(&mut client);
    send(&mut client, 1);
    let options = with_classless(&[24, 198, 51, 100, 0, 0, 0, 0]);
    let ack = reply(5, xid, HOST, SERVER, &options);
    let Event::Bound(lease) = client
        .receive(&server_frame(&ack, SERVER, HOST, CLIENT), 2)
        .unwrap()
    else {
        panic!("expected accepted classless ACK");
    };
    assert_eq!(
        lease.router, None,
        "no legacy default if option 121 lacks /0"
    );

    let xid = transaction(&send(&mut client, lease.renew_at));
    let ack = reply(5, xid, HOST, SERVER, &[51, 4, 0, 0, 0, 16]);
    let Event::Bound(renewed) = client
        .receive(
            &server_frame(&ack, SERVER, HOST, CLIENT),
            lease.renew_at + 1,
        )
        .unwrap()
    else {
        panic!("expected accepted renewal");
    };
    assert_eq!(renewed.routes, lease.routes);
    assert_eq!(renewed.router, None);

    let xid = transaction(&send(&mut client, renewed.rebind_at));
    let legacy = reply(5, xid, HOST, OTHER, &offer_options(LIFETIME));
    let Event::Bound(rebound) = client
        .receive(
            &server_frame(&legacy, OTHER, HOST, CLIENT),
            renewed.rebind_at + 1,
        )
        .unwrap()
    else {
        panic!("expected new server lease");
    };
    assert_eq!(rebound.router, Some(SERVER));
    assert_eq!(rebound.routes, None);
}

#[test]
fn invalid_classless_routes_cannot_publish_an_ack() {
    let mut client = Client::new(CLIENT, 37).unwrap();
    let lease = bound(&mut client);
    let xid = transaction(&send(&mut client, lease.renew_at));
    let duplicate = [24, 198, 51, 100, 192, 0, 2, 1].repeat(2);
    let too_many = [
        8, 1, 192, 0, 2, 1, 8, 2, 192, 0, 2, 1, 8, 3, 192, 0, 2, 1, 8, 4, 192, 0, 2, 1, 8, 5, 192,
        0, 2, 1, 8, 6, 192, 0, 2, 1, 8, 7, 192, 0, 2, 1, 8, 8, 192, 0, 2, 1,
    ];
    for (entries, error) in [
        (&[][..], Error::Length),
        (&[33, 192, 0, 2, 1], Error::Header),
        (&[24, 198, 51, 100, 192, 0, 2], Error::Length),
        (&[25, 198, 51, 100, 129, 192, 0, 2, 1], Error::Header),
        (&[0, 198, 51, 100, 1], Error::Address),
        (&[24, 192, 0, 2, 192, 0, 2, 1], Error::Header),
        (&[24, 192, 0, 2, 0, 0, 0, 0], Error::Header),
        (&duplicate, Error::Header),
        (&too_many, Error::Capacity),
        (&[0; (ROUTE_CAPACITY - 1) * 9 + 1], Error::Capacity),
    ] {
        let options = with_classless(entries);
        let ack = reply(5, xid, HOST, SERVER, &options);
        assert_eq!(
            client.receive(
                &server_frame(&ack, SERVER, HOST, CLIENT),
                lease.renew_at + 1
            ),
            Err(error),
            "routes {entries:?}",
        );
        assert_eq!(client.lease(), Some(lease));
        assert_eq!(client.state(), State::Renewing);
    }
}
