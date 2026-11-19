//! Exercise the real DNS resolver and TCP connection over native packet I/O.
use super::*;
use dunit_net_core::{
    dns::Answer,
    tcp::{table::{Table, MAX_STREAMS}, Endpoint, State, StreamError},
    Egress,
};

const FALLBACK_NAME: &[u8] = b"tcp.du.net.test";
use crate::tcp_listener::{tcp_error, Echoes};

pub fn exchange(
    queue: &Queue,
    peer: &mut Peer,
    mtu: usize,
    started: Instant,
    rx: &mut [u8],
    tx: &mut [u8],
) -> Result<Answer> {
    let server = Address { ip: HOST, port: 53 };
    let mut resolver = Resolver::new_with_tcp(peer, server).map_err(net_error)?;
    let now = clock(started)?;
    require(
        resolver.start(FALLBACK_NAME, Kind::A, now).map_err(net_error)?.is_none(),
        "fresh fallback query",
    )?;
    let Poll::Frame(query) = resolver.poll(peer, now).map_err(net_error)? else {
        return Err(Error(errno::EIO));
    };
    send_frame(queue, &query, tx)?;
    queue.wait(wait_set::READ, RX_TIMEOUT_NS)?;
    let count = queue.receive(rx)?;
    for frame in Records::new(&rx[..count]).map_err(wire_error)? {
        peer.receive(frame, clock(started)?).map_err(net_error)?;
    }
    let now = clock(started)?;
    let Poll::TcpFallback { server: endpoint, query, deadline_ms } =
        resolver.poll(peer, now).map_err(net_error)?
    else {
        return Err(Error(errno::EIO));
    };
    require(endpoint == server, "DNS fallback endpoint")?;

    let mut table = Table::new(GUEST, mtu, 0x10000000 ^ process::id()).map_err(tcp_error)?;
    let mut echo = Echoes::listen(&mut table)?;
    let mut client = Some(table.connect(Endpoint { ip: HOST, port: server.port }, now).map_err(tcp_error)?);
    let finish_by = deadline_ms.saturating_add(10_000);
    let mut answer = None;
    let mut submitted = false;
    let mut stream = [0; 128];
    loop {
        let now = clock(started)?;
        if now >= finish_by || (answer.is_none() && now >= deadline_ms) {
            return Err(Error(errno::EAGAIN));
        }
        if let Some(id) = client {
            if table.state(id).map_err(tcp_error)? == State::Established && !submitted {
                require(table.send(id, &query).map_err(tcp_error)? == query.len(), "TCP query send")?;
                submitted = true;
            }
            if submitted && answer.is_none() {
                match table.read(id, &mut stream) {
                    Ok(0) => {
                        resolver.tcp_eof(server, now).map_err(net_error)?;
                        return Err(Error(errno::EIO));
                    }
                    Ok(count) => {
                        answer = resolver.receive_tcp(server, &stream[..count], now).map_err(net_error)?;
                        if answer.is_some() {
                            table.shutdown_write(id).map_err(tcp_error)?;
                        }
                    }
                    Err(StreamError::WouldBlock) => {}
                    Err(error) => return Err(tcp_error(error)),
                }
            }
            if table.state(id).map_err(tcp_error)? == State::Closed {
                require(answer.is_some() && !table.readiness(id).map_err(tcp_error)?.error, "DNS TCP close")?;
                table.close(id).map_err(tcp_error)?;
                client = None;
            }
        }

        echo.step(&mut table)?;
        if client.is_none() && echo.complete() {
            echo.close(&mut table)?;
            resolver.close(peer).map_err(net_error)?;
            return answer.ok_or(Error(errno::EIO));
        }

        for _ in 0..MAX_STREAMS {
            let Some(packet) = table.poll(now).map_err(tcp_error)? else { break };
            match peer.tcp_egress(packet.source, packet.destination, &packet.bytes, now).map_err(net_error)? {
                Egress::Sent(frame) | Egress::Neighbor(frame) => send_frame(queue, &frame, tx)?,
                Egress::Waiting => {} // TCP's timer retries once ARP resolves.
            }
        }
        let phase_limit = if answer.is_none() { deadline_ms } else { finish_by };
        let deadline = table.deadline().unwrap_or(phase_limit).min(phase_limit);
        let timeout = deadline.saturating_sub(clock(started)?).saturating_mul(1_000_000);
        match queue.wait(wait_set::READ, timeout) {
            Err(Error(errno::EAGAIN)) => continue,
            Err(error) => return Err(error),
            Ok(()) => {}
        }
        let count = queue.receive(rx)?;
        for frame in Records::new(&rx[..count]).map_err(wire_error)? {
            let now = clock(started)?;
            if let Some(ip) = peer.tcp_ingress(frame).map_err(net_error)? {
                table.receive(ip.source, ip.destination, ip.payload, now).map_err(tcp_error)?;
            } else if let Some(response) = peer.receive(frame, now).map_err(net_error)? {
                send_frame(queue, &response, tx)?;
            }
        }
    }
}
