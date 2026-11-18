//! Shared deterministic native/host logical-peer scenario; every reported count
//! follows parsing and checking the returned wire bytes, never a device claim.
use super::*;
#[derive(Debug, PartialEq, Eq)]
pub struct Report {
    pub echo_replies: u16,
    pub arp_requests: u16,
    pub rejected: u16,
}
fn verify_echo(b: &[u8], sequence: u16, data: &[u8]) -> Result<(), Error> {
    let f = Ethernet::parse(b)?;
    let ip = Ipv4::parse(f.payload)?;
    let e = Echo::parse(ip.payload)?;
    if f.destination != [2, 0, 0, 0, 0, 1]
        || f.source != [2, 0, 0, 0, 0, 2]
        || ip.source != [192, 0, 2, 2]
        || ip.destination != [192, 0, 2, 1]
        || !e.reply
        || e.identifier != 0x1234
        || e.sequence != sequence
        || e.data != data
    {
        return Err(Error::Header);
    }
    Ok(())
}
fn reject<T>(r: Result<T, Error>, expected: Error, report: &mut Report) -> Result<(), Error> {
    match r {
        Err(e) if e == expected => {
            report.rejected += 1;
            Ok(())
        }
        _ => Err(Error::Header),
    }
}
pub fn run() -> Result<Report, Error> {
    let mut a = Peer::new([2, 0, 0, 0, 0, 1], [192, 0, 2, 1])?;
    let mut b = Peer::new([2, 0, 0, 0, 0, 2], [192, 0, 2, 2])?;
    let mut report = Report {
        echo_replies: 0,
        arp_requests: 0,
        rejected: 0,
    };
    for (round, now) in [(0u16, 0u64), (1, 1), (2, ARP_REACHABLE_MS + 1)] {
        let mut request = a
            .echo(b.ip, 0x1234, round, b"odd bytes", now)?
            .ok_or(Error::Header)?;
        if Ethernet::parse(&request)?.kind == 0x0806 {
            report.arp_requests += 1;
            let response = b.receive(&request, now)?.ok_or(Error::Header)?;
            a.receive(&response, now)?;
            request = a
                .echo(b.ip, 0x1234, round, b"odd bytes", now)?
                .ok_or(Error::Header)?;
        }
        let response = b.receive(&request, now)?.ok_or(Error::Header)?;
        verify_echo(&response, round, b"odd bytes")?;
        report.echo_replies += 1;
        let mut corrupt = request.clone();
        corrupt[24] ^= 1;
        reject(b.receive(&corrupt, now), Error::Checksum, &mut report)?;
    }
    if report.arp_requests != 2 {
        return Err(Error::Header);
    }
    let absent = [192, 0, 2, 99];
    for n in 0..ARP_ATTEMPTS {
        let frame = a
            .echo(absent, 1, 1, &[], 40_000 + n as u64 * ARP_RETRY_MS)?
            .ok_or(Error::Header)?;
        if Arp::parse(Ethernet::parse(&frame)?.payload)?.target_ip != absent {
            return Err(Error::Header);
        }
        report.arp_requests += 1;
    }
    reject(
        a.echo(absent, 1, 1, &[], 43_000),
        Error::Timeout,
        &mut report,
    )?;
    let unsolicited = Arp {
        operation: 2,
        sender_mac: b.mac,
        sender_ip: b.ip,
        target_mac: a.mac,
        target_ip: a.ip,
    };
    reject(
        a.neighbors.accept(unsolicited, b.mac, a.mac, a.ip, 43_000),
        Error::Unsolicited,
        &mut report,
    )?;
    Ok(report)
}
