//! Exercise the real DNS resolver and TCP connection over native packet I/O.
use super::*;
use dunit_net_core::{
    dns::Answer,
    tcp::{Connection, Endpoint, State, StreamError},
    Mac,
};

const FALLBACK_NAME: &[u8] = b"tcp.du.net.test";
const HOST_MAC: Mac = [2, 0, 0, 0, 0, 1];
const CLIENT_PORT: u16 = 49153;

fn tcp_error(_: StreamError) -> Error {
    Error(errno::EIO)
}

pub fn exchange(
    queue: &Queue,
    peer: &mut Peer,
    mac: Mac,
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

    let mut conn = Connection::connect(
        Endpoint { ip: GUEST, port: CLIENT_PORT },
        Endpoint { ip: HOST, port: server.port },
        0x10000000 ^ process::id(),
        mtu,
        now,
    ).map_err(tcp_error)?;
    let mut answer = None;
    let mut submitted = false;
    let mut stream = [0; 128];
    loop {
        let now = clock(started)?;
        if now >= deadline_ms {
            return Err(Error(errno::EAGAIN));
        }
        if conn.state() == State::Established && !submitted {
            require(conn.send(&query).map_err(tcp_error)? == query.len(), "TCP query send")?;
            submitted = true;
        }
        if submitted && answer.is_none() {
            match conn.read(&mut stream) {
                Ok(0) => {
                    resolver.tcp_eof(server, now).map_err(net_error)?;
                    return Err(Error(errno::EIO));
                }
                Ok(count) => {
                    answer = resolver.receive_tcp(server, &stream[..count], now).map_err(net_error)?;
                    if answer.is_some() {
                        conn.shutdown_write().map_err(tcp_error)?;
                    }
                }
                Err(StreamError::WouldBlock) => {}
                Err(error) => return Err(tcp_error(error)),
            }
        }
        if let Some(segment) = conn.poll(now).map_err(tcp_error)? {
            let packet = Ipv4::encode_protocol(GUEST, HOST, 6, &segment).map_err(net_error)?;
            let frame = Ethernet::encode(HOST_MAC, mac, 0x0800, &packet).map_err(net_error)?;
            send_frame(queue, &frame, tx)?;
        }
        if conn.state() == State::Closed {
            resolver.close(peer).map_err(net_error)?;
            return answer.ok_or(Error(errno::EIO));
        }

        let deadline = conn.deadline().unwrap_or(deadline_ms).min(deadline_ms);
        let timeout = deadline.saturating_sub(clock(started)?).saturating_mul(1_000_000);
        match queue.wait(wait_set::READ, timeout) {
            Err(Error(errno::EAGAIN)) => continue,
            Err(error) => return Err(error),
            Ok(()) => {}
        }
        let count = queue.receive(rx)?;
        for frame in Records::new(&rx[..count]).map_err(wire_error)? {
            let frame = Ethernet::parse(frame).map_err(net_error)?;
            require(frame.source == HOST_MAC && frame.destination == mac, "TCP frame MAC")?;
            let ip = Ipv4::parse(frame.payload).map_err(net_error)?;
            require(frame.kind == 0x0800 && ip.protocol == 6, "TCP frame protocol")?;
            conn.receive(ip.source, ip.destination, ip.payload, clock(started)?).map_err(tcp_error)?;
        }
    }
}
