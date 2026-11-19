//! Deterministic two-ended wire/connection exercise shared by host and native tests.
use super::*;
use crate::{Ethernet, Ipv4, Mac};

const CLIENT: Endpoint = Endpoint {
    ip: [192, 0, 2, 2],
    port: 49152,
};
const SERVER: Endpoint = Endpoint {
    ip: [192, 0, 2, 3],
    port: 8000,
};
const CLIENT_MAC: Mac = [2, 0, 0, 0, 0, 2];
const SERVER_MAC: Mac = [2, 0, 0, 0, 0, 3];

pub const STREAM_BYTES: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    pub bytes: usize,
    pub retries: usize,
}

pub fn run() -> Result<Report> {
    exercise(STREAM_BYTES)
}

fn outgoing(connection: &mut Connection, now: u64) -> Result<Vec<u8>> {
    connection.poll(now)?.ok_or(StreamError::WouldBlock)
}

fn exchange(from: &mut Connection, to: &mut Connection, now: u64) -> Result<()> {
    let segment = outgoing(from, now)?;
    let (source, destination) = from.endpoints();
    let ip = Ipv4::encode_protocol(source.ip, destination.ip, 6, &segment)?;
    let (source_mac, destination_mac) = if source == CLIENT {
        (CLIENT_MAC, SERVER_MAC)
    } else {
        (SERVER_MAC, CLIENT_MAC)
    };
    let bytes = Ethernet::encode(destination_mac, source_mac, 0x0800, &ip)?;
    let frame = Ethernet::parse(&bytes)?;
    let received = Ipv4::parse(frame.payload)?;
    to.receive(received.source, received.destination, received.payload, now)
}

fn verify(ok: bool) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error::Header.into())
    }
}

pub(super) fn exercise(bytes: usize) -> Result<Report> {
    let mut now = 0;
    let mut client = Connection::connect(CLIENT, SERVER, u32::MAX - 3, MTU, now)?;
    let lost = outgoing(&mut client, now)?;
    verify(client.poll(INITIAL_RTO - 1)?.is_none())?;
    now += INITIAL_RTO;
    let duplicate = outgoing(&mut client, now)?;
    verify(duplicate == lost)?;
    let mut server = Connection::accept(SERVER, CLIENT, 23, MTU, now, &duplicate)?;
    exchange(&mut server, &mut client, now)?;
    exchange(&mut client, &mut server, now)?;
    verify(client.state() == State::Established && server.state() == State::Established)?;
    let mut output = [0u8; 509];
    let mut input = [0u8; 509];
    let mut sent = 0;
    let mut retries = 1;
    while sent < bytes {
        let size = (bytes - sent).min(output.len());
        for (index, byte) in output[..size].iter_mut().enumerate() {
            *byte = (sent + index).wrapping_mul(137) as u8;
        }
        verify(client.send(&output[..size])? == size)?;
        if sent == 0 {
            let lost = outgoing(&mut client, now)?;
            let deadline = client.deadline().ok_or(StreamError::WouldBlock)?;
            verify(client.poll(deadline - 1)?.is_none())?;
            now = deadline;
            let repaired = outgoing(&mut client, now)?;
            verify(lost == repaired)?;
            server.receive(CLIENT.ip, SERVER.ip, &repaired, now)?;
            server.receive(CLIENT.ip, SERVER.ip, &repaired, now)?;
            retries += 1;
        } else {
            exchange(&mut client, &mut server, now)?;
        }
        verify(server.read(&mut input)? == size && input[..size] == output[..size])?;
        exchange(&mut server, &mut client, now)?;
        sent += size;
        now += 1;
    }
    client.shutdown_write()?;
    exchange(&mut client, &mut server, now)?;
    verify(server.state() == State::CloseWait && server.read(&mut input)? == 0)?;
    // A half-closed peer can still send in the other direction.
    verify(server.send(b"ok")? == 2)?;
    exchange(&mut server, &mut client, now)?;
    verify(client.read(&mut input)? == 2 && &input[..2] == b"ok")?;
    exchange(&mut client, &mut server, now)?;
    server.shutdown_write()?;
    exchange(&mut server, &mut client, now)?;
    verify(client.state() == State::TimeWait && client.read(&mut input)? == 0)?;
    exchange(&mut client, &mut server, now)?;
    verify(server.state() == State::Closed)?;
    verify(
        client.poll(now.saturating_add(TIME_WAIT))?.is_none() && client.state() == State::Closed,
    )?;
    Ok(Report {
        bytes: sent,
        retries,
    })
}
