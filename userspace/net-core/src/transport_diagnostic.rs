//! Shared native/host UDP and DNS exchange: every success follows actual frames.
use super::dns::{AddressData, Answer, Kind, Poll, Question, Resolver};
use super::udp::{Address, Send, QUEUE_DEPTH};
use super::*;
#[derive(Debug, PartialEq, Eq)]
pub struct Report {
    pub datagrams: u16,
    pub dns_answers: u16,
    pub rejected: u16,
}
fn check(value: bool) -> Result<(), Error> {
    if value {
        Ok(())
    } else {
        Err(Error::Header)
    }
}
fn frame(poll: Poll) -> Result<Vec<u8>, Error> {
    match poll {
        Poll::Frame(b) => Ok(b),
        _ => Err(Error::Header),
    }
}
fn sent(send: Send) -> Result<Vec<u8>, Error> {
    match send {
        Send::Sent(b) => Ok(b),
        _ => Err(Error::Header),
    }
}
pub fn run() -> Result<Report, Error> {
    let ia = [192, 0, 2, 1];
    let ib = [192, 0, 2, 2];
    let mask = [255, 255, 255, 0];
    let mut a = Peer::new([2, 0, 0, 0, 0, 1], ia, mask, MTU)?;
    let mut b = Peer::new([2, 0, 0, 0, 0, 2], ib, mask, MTU)?;
    let server = Address { ip: ib, port: 53 };
    b.udp_bind(53)?;
    let mut resolver = Resolver::new(&mut a, server)?;
    let expected = Answer {
        address: AddressData::V4([203, 0, 113, 7]),
        ttl: 2,
    };
    check(resolver.start(b"Example.test", Kind::A, 0)?.is_none())?;
    // First frame is ARP, not a transmitted DNS query.
    let arp = frame(resolver.poll(&mut a, 0)?)?;
    check(Ethernet::parse(&arp)?.kind == 0x0806)?;
    let reply = b.receive(&arp, 0)?.ok_or(Error::Header)?;
    a.receive(&reply, 0)?;
    let query = frame(resolver.poll(&mut a, 0)?)?;
    b.receive(&query, 0)?;
    let request = b.udp_recv(53)?;
    let question = Question::parse(&request.data)?;
    check(question.name == b"example.test")?;
    // Resolve the reverse neighbor using an ordinary UDP attempt.
    let response = question.response(&expected)?;
    let reverse = match b.udp_send(53, Some(request.source), &response, 0)? {
        Send::Neighbor(f) => f,
        _ => return Err(Error::Header),
    };
    let reverse_reply = a.receive(&reverse, 0)?.ok_or(Error::Header)?;
    b.receive(&reverse_reply, 0)?;
    // Responses with another transaction ID or a compression cycle cannot
    // complete this transaction, and leave it available for the valid response.
    let mut wrong = response.clone();
    put(&mut wrong, 0, question.id.wrapping_add(1));
    a.receive(&sent(b.udp_send(53, Some(request.source), &wrong, 0)?)?, 0)?;
    check(resolver.poll(&mut a, 0) == Err(Error::Unsolicited))?;
    let mut malformed = response.clone();
    malformed[12] = 0xc0;
    malformed[13] = 12;
    a.receive(
        &sent(b.udp_send(53, Some(request.source), &malformed, 0)?)?,
        0,
    )?;
    check(resolver.poll(&mut a, 0) == Err(Error::Header))?;
    let reply = sent(b.udp_send(53, Some(request.source), &response, 0)?)?;
    a.receive(&reply, 0)?;
    check(resolver.poll(&mut a, 0)? == Poll::Answer(expected.clone()))?;
    let mut report = Report {
        datagrams: 4,
        dns_answers: 1,
        rejected: 2,
    };
    let cached = resolver
        .start(b"EXAMPLE.TEST.", Kind::A, 1000)?
        .ok_or(Error::Header)?;
    check(cached.address == expected.address && cached.ttl == 1)?;
    report.dns_answers += 1;
    // CNAME and AAAA use the same UDP transaction path as the first A answer.
    for kind in [Kind::A, Kind::Aaaa] {
        let name: &[u8] = if kind == Kind::A {
            b"alias.test"
        } else {
            b"ipv6.test"
        };
        check(resolver.start(name, kind, 1000)?.is_none())?;
        b.receive(&frame(resolver.poll(&mut a, 1000)?)?, 1000)?;
        let request = b.udp_recv(53)?;
        let q = Question::parse(&request.data)?;
        let expected = if kind == Kind::A {
            Answer {
                address: AddressData::V4([203, 0, 113, 9]),
                ttl: 3,
            }
        } else {
            Answer {
                address: AddressData::V6([0x20; 16]),
                ttl: 4,
            }
        };
        let response = if kind == Kind::A {
            let mut bytes = q.encode()?;
            put(&mut bytes, 2, 0x8180);
            put(&mut bytes, 6, 2);
            let alias_offset = bytes.len() + 12;
            bytes.extend_from_slice(&[0xc0, 12, 0, 5, 0, 1, 0, 0, 0, 3, 0, 13]);
            bytes.extend_from_slice(b"\x06target\x04test\x00");
            bytes.extend_from_slice(&[
                0xc0,
                alias_offset as u8,
                0,
                1,
                0,
                1,
                0,
                0,
                0,
                9,
                0,
                4,
                203,
                0,
                113,
                9,
            ]);
            bytes
        } else {
            q.response(&expected)?
        };
        a.receive(
            &sent(b.udp_send(53, Some(request.source), &response, 1000)?)?,
            1000,
        )?;
        check(resolver.poll(&mut a, 1000)? == Poll::Answer(expected))?;
        report.datagrams += 2;
        report.dns_answers += 1;
    }
    // Expiry starts a fresh transaction; lost replies cause exactly three sends.
    check(resolver.start(b"example.test", Kind::A, 2000)?.is_none())?;
    for now in [2000, 3000, 4000] {
        let query = frame(resolver.poll(&mut a, now)?)?;
        check(Ipv4::parse(Ethernet::parse(&query)?.payload)?.protocol == 17)?;
        b.receive(&query, now)?;
        b.udp_recv(53)?;
        report.datagrams += 1;
    }
    check(resolver.poll(&mut a, 5000) == Err(Error::Timeout))?;
    report.rejected += 1;
    // Overflow drops only the new datagram; FIFO and truncation are observable.
    let port = a.udp_bind(9000)?;
    for n in 0..QUEUE_DEPTH {
        let frame =
            sent(b.udp_send(53, Some(Address { ip: ia, port }), &[n as u8, 9, 8], 5000)?)?;
        a.receive(&frame, 5000)?;
        report.datagrams += 1;
    }
    let mut packet = sent(b.udp_send(53, Some(Address { ip: ia, port }), b"overflow", 5000)?)?;
    check(a.receive(&packet, 5000) == Err(Error::Capacity))?;
    report.rejected += 1;
    // Same framing but bad UDP checksum must fail before queue mutation.
    packet[42] ^= 1;
    check(a.receive(&packet, 5000) == Err(Error::Checksum))?;
    report.rejected += 1;
    for n in 0..QUEUE_DEPTH {
        let mut out = [0u8; 1];
        let received = a.udp_recv_into(port, &mut out)?;
        check(
            out[0] == n as u8
                && received.copied == 1
                && received.original_len == 3
                && received.source == server,
        )?;
    }
    check(!a.udp_ready(port)? && a.udp_recv(port) == Err(Error::WouldBlock))?;
    report.rejected += 1;
    // NXDOMAIN and TC are explicit terminal errors, with no fabricated answer.
    for (name, flags, expected) in [
        (b"missing.test".as_slice(), 0x8183, Error::NameNotFound),
        (b"truncated.test".as_slice(), 0x8380, Error::Unsupported),
    ] {
        resolver.start(name, Kind::A, 5000)?;
        b.receive(&frame(resolver.poll(&mut a, 5000)?)?, 5000)?;
        let request = b.udp_recv(53)?;
        let mut response = Question::parse(&request.data)?.encode()?;
        put(&mut response, 2, flags);
        a.receive(
            &sent(b.udp_send(53, Some(request.source), &response, 5000)?)?,
            5000,
        )?;
        check(resolver.poll(&mut a, 5000) == Err(expected))?;
        report.datagrams += 2;
        report.rejected += 1;
    }
    resolver.close(&mut a)?;
    a.udp_close(port)?;
    Ok(report)
}
