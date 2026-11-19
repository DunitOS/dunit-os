use super::*;

const ADDRESS: Ip = [192, 0, 2, 10];
const OTHER: Ip = [192, 0, 2, 20];
const LOCAL: Mac = [2, 0, 0, 0, 0, 1];
const FOREIGN: Mac = [2, 0, 0, 0, 0, 2];

fn arp(operation: u16, from: Mac, l2: Mac, source: Ip, target: Ip) -> Vec<u8> {
    let bytes = Arp {
        operation,
        sender_mac: from,
        sender_ip: source,
        target_mac: [0; 6],
        target_ip: target,
    }
    .encode_framing();
    Ethernet::encode(l2, from, 0x0806, &bytes).unwrap()
}

#[test]
fn three_spaced_probes_quiet_interval_and_two_announcements() {
    let start = 100;
    let mut probe = Probe::new(LOCAL, ADDRESS, 33, start).unwrap();
    let mut due = probe.deadline().unwrap();
    assert!((start..=start + PROBE_WAIT_MS).contains(&due));
    if due > start {
        assert_eq!(probe.poll(due - 1), Ok(Action::Idle));
    }
    for index in 0..PROBES {
        let Action::Probe(bytes) = probe.poll(due).unwrap() else {
            panic!("expected probe");
        };
        assert_eq!(bytes, arp(1, LOCAL, BROADCAST, UNSPECIFIED, ADDRESS));
        let packet = Ethernet::parse(&bytes).unwrap();
        assert_eq!(Arp::parse(packet.payload), Err(Error::Address));
        let next = probe.deadline().unwrap();
        if index + 1 == PROBES {
            assert_eq!(next - due, ANNOUNCE_WAIT_MS);
        } else {
            assert!((PROBE_MIN_MS..=PROBE_MAX_MS).contains(&(next - due)));
        }
        due = next;
    }
    assert_eq!(probe.poll(due - 1), Ok(Action::Idle));
    let claim = arp(1, LOCAL, BROADCAST, ADDRESS, ADDRESS);
    assert_eq!(probe.poll(due), Ok(Action::Claim(claim.clone())));
    assert_eq!(probe.deadline(), Some(due + ANNOUNCE_INTERVAL_MS));
    assert_eq!(
        probe.poll(due + ANNOUNCE_INTERVAL_MS),
        Ok(Action::Announce(claim))
    );
    assert_eq!(probe.deadline(), None);
    assert_eq!(probe.poll(due + 5000), Ok(Action::Idle));
}

#[test]
fn late_poll_spaces_probes_and_regressing_clock_fails() {
    let mut probe = Probe::new(LOCAL, ADDRESS, 33, 0).unwrap();
    assert!(matches!(probe.poll(25_000), Ok(Action::Probe(_))));
    assert_eq!(probe.poll(24_999), Err(Error::Header));
    assert_eq!(probe.poll(25_000), Ok(Action::Idle));
    assert!((26_000..=27_000).contains(&probe.deadline().unwrap()));
}

#[test]
fn reject_invalid_input_without_reporting_false_conflicts() {
    assert!(matches!(
        Probe::new([0; 6], ADDRESS, 1, 0),
        Err(Error::Address)
    ));
    for address in [UNSPECIFIED, [127, 0, 0, 1], [224, 1, 1, 1], [255; 4]] {
        assert!(matches!(
            Probe::new(LOCAL, address, 1, 0),
            Err(Error::Address)
        ));
    }
    let probe = Probe::new(LOCAL, ADDRESS, 1, 0).unwrap();
    let mut forged = arp(1, FOREIGN, BROADCAST, ADDRESS, OTHER);
    forged[14 + 8] ^= 4;
    let wrong_target_mac = arp(1, FOREIGN, [2, 0, 0, 0, 0, 3], ADDRESS, OTHER);
    let bad_operation = arp(0, FOREIGN, BROADCAST, ADDRESS, OTHER);
    let wrong_ip = arp(1, FOREIGN, BROADCAST, UNSPECIFIED, OTHER);
    let replied_unspecified = arp(2, FOREIGN, BROADCAST, UNSPECIFIED, ADDRESS);
    let own = arp(1, LOCAL, BROADCAST, ADDRESS, OTHER);
    let short = arp(1, FOREIGN, BROADCAST, ADDRESS, OTHER);
    for packet in [
        forged.as_slice(),
        &wrong_target_mac,
        &bad_operation,
        &wrong_ip,
        &replied_unspecified,
        &own,
        &short[..59],
    ] {
        assert!(!probe.observe(packet));
    }
}

#[test]
fn detect_claims_and_simultaneous_probes_only_while_tentative() {
    let mut probe = Probe::new(LOCAL, ADDRESS, 1, 0).unwrap();
    let simultaneous = arp(1, FOREIGN, BROADCAST, UNSPECIFIED, ADDRESS);
    let announcement = arp(1, FOREIGN, BROADCAST, ADDRESS, ADDRESS);
    let reply = arp(2, FOREIGN, LOCAL, ADDRESS, OTHER);
    let wants_address = arp(1, FOREIGN, BROADCAST, OTHER, ADDRESS);
    for _ in 0..PROBES {
        let due = probe.deadline().unwrap();
        assert!(probe.observe(&simultaneous));
        assert!(probe.observe(&announcement));
        assert!(probe.observe(&reply));
        assert!(!probe.observe(&wants_address));
        probe.poll(due).unwrap();
    }
    assert!(probe.observe(&simultaneous));
    assert!(matches!(
        probe.poll(probe.deadline().unwrap()),
        Ok(Action::Claim(_))
    ));
    assert!(!probe.observe(&simultaneous));
    assert!(probe.observe(&announcement));
    assert!(probe.observe(&reply));
}

#[test]
fn active_address_replies_to_probe_and_probe_observes_conflicting_claim() {
    let tentative = Probe::new(FOREIGN, ADDRESS, 1, 0).unwrap();
    let incoming = arp(1, FOREIGN, BROADCAST, UNSPECIFIED, ADDRESS);
    let response = reply_to_probe(LOCAL, ADDRESS, &incoming).unwrap().unwrap();
    let frame = Ethernet::parse(&response).unwrap();
    assert_eq!((frame.source, frame.destination), (LOCAL, FOREIGN));
    assert_eq!(Arp::parse(frame.payload), Err(Error::Address));
    assert_eq!(
        Arp::parse_framing(frame.payload),
        Ok(Arp {
            operation: 2,
            sender_mac: LOCAL,
            sender_ip: ADDRESS,
            target_mac: FOREIGN,
            target_ip: UNSPECIFIED,
        })
    );
    assert!(tentative.observe(&response));
    let mut forged = response;
    forged[14 + 18] ^= 4;
    assert!(!tentative.observe(&forged));
    assert_eq!(reply_to_probe(LOCAL, OTHER, &incoming), Ok(None));
    let ordinary = arp(1, FOREIGN, BROADCAST, OTHER, ADDRESS);
    assert_eq!(reply_to_probe(LOCAL, ADDRESS, &ordinary), Ok(None));
    let own = arp(1, LOCAL, BROADCAST, UNSPECIFIED, ADDRESS);
    assert_eq!(reply_to_probe(LOCAL, ADDRESS, &own), Ok(None));
}
