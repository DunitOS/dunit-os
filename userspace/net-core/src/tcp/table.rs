//! Bounded TCPv4 admission, demultiplexing and stream ownership for one
//! interface. A service still owns route selection and listener policy.
use super::{
    Connection, Endpoint, Result, Segment, State, StreamError, ACK, FIN, QUEUE_BYTES, RST, SYN,
};
use crate::{valid_ip, Error, Ip, MTU};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

pub const MAX_STREAMS: usize = 8;
pub const MAX_LISTENERS: usize = 4;
pub const MAX_PENDING: usize = 4;
const FIRST_EPHEMERAL: u16 = 49152;
const ADMISSION_MS: u64 = 30_000;
const DETACHED_MS: u64 = 120_000;
// A replaced interface must not make IDs from an older Table valid again.
static GENERATIONS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamId {
    slot: usize,
    generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListenerId {
    slot: usize,
    generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Readiness {
    /// A read returns bytes or EOF immediately.
    pub readable: bool,
    /// This stream still accepts some bytes into its bounded local queue.
    pub writable: bool,
    pub hangup: bool,
    pub error: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Outbound {
    pub source: Endpoint,
    pub destination: Endpoint,
    pub bytes: Vec<u8>,
}

impl Outbound {
    fn ack_only(&self) -> bool {
        self.bytes.len() == super::wire::HEADER && self.bytes[13] == ACK
    }

    /// A newer packet replaces an unsent packet of the same class for its
    /// four-tuple. A pure ACK and a sequence-consuming flight can coexist.
    pub fn same_pending_slot(&self, other: &Self) -> bool {
        (self.source, self.destination) == (other.source, other.destination)
            && self.ack_only() == other.ack_only()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ingress {
    Delivered,
    Admitted,
    Unmatched,
    BacklogFull,
}

struct Listener {
    generation: u64,
    port: u16,
    backlog: usize,
}

enum Owner {
    Pending { listener: ListenerId, deadline: u64 },
    Held,
    Detached { deadline: u64 },
}

struct Stream {
    generation: u64,
    owner: Owner,
    connection: Connection,
}

impl Stream {
    fn pending_for(&self, id: ListenerId) -> bool {
        matches!(self.owner, Owner::Pending { listener, .. } if listener == id)
    }

    fn owner_deadline(&self) -> Option<u64> {
        match self.owner {
            Owner::Pending { deadline, .. } | Owner::Detached { deadline } => Some(deadline),
            Owner::Held => None,
        }
    }
}

pub struct Table {
    local_ip: Ip,
    mtu: usize,
    sequence_seed: u32,
    next_port: u16,
    now: u64,
    next_poll: usize,
    listeners: [Option<Listener>; MAX_LISTENERS],
    streams: [Option<Stream>; MAX_STREAMS],
}

impl Table {
    /// The seed diversifies initial sequences. Production callers must provide
    /// their own entropy before using this as an Internet-facing stack.
    pub fn new(local_ip: Ip, mtu: usize, seed: u32) -> Result<Self> {
        if !valid_ip(local_ip) {
            return Err(Error::Address.into());
        }
        if !(44..=MTU).contains(&mtu) {
            return Err(Error::Length.into());
        }
        Ok(Self {
            local_ip,
            mtu,
            sequence_seed: seed.max(1),
            next_port: FIRST_EPHEMERAL,
            now: 0,
            next_poll: 0,
            listeners: core::array::from_fn(|_| None),
            streams: core::array::from_fn(|_| None),
        })
    }

    pub fn listen(&mut self, port: u16, backlog: usize) -> Result<ListenerId> {
        if port == 0 || backlog == 0 {
            return Err(Error::Address.into());
        }
        if backlog > MAX_PENDING {
            return Err(Error::Capacity.into());
        }
        if self.port_in_use(port) {
            return Err(Error::PortInUse.into());
        }
        let slot = self
            .listeners
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity)?;
        let generation = Self::generation()?;
        self.listeners[slot] = Some(Listener {
            generation,
            port,
            backlog,
        });
        Ok(ListenerId { slot, generation })
    }

    pub fn listener_ready(&self, id: ListenerId) -> Result<bool> {
        self.listener(id)?;
        Ok(self.ready_slot(id).is_some())
    }

    /// Take the oldest eligible SYN. An unfinished handshake never blocks an
    /// established stream behind it. A remote half-close before accept is
    /// preserved, including buffered bytes and the subsequent EOF.
    pub fn accept(&mut self, id: ListenerId) -> Result<StreamId> {
        self.listener(id)?;
        let slot = self.ready_slot(id).ok_or(StreamError::WouldBlock)?;
        let stream = self.streams[slot].as_mut().unwrap();
        stream.owner = Owner::Held;
        Ok(StreamId {
            slot,
            generation: stream.generation,
        })
    }

    /// Cancel unaccepted handshakes and connections. Accepted streams retain
    /// their four-tuples and continue independently.
    pub fn close_listener(&mut self, id: ListenerId) -> Result<()> {
        self.listener(id)?;
        self.listeners[id.slot] = None;
        for slot in &mut self.streams {
            if slot.as_ref().is_some_and(|stream| stream.pending_for(id)) {
                *slot = None;
            }
        }
        Ok(())
    }

    pub fn connect(&mut self, remote: Endpoint, now: u64) -> Result<StreamId> {
        remote.validate()?;
        self.advance(now)?;
        let slot = self.free_stream()?;
        let port = self.ephemeral_port()?;
        let generation = Self::generation()?;
        let local = Endpoint {
            ip: self.local_ip,
            port,
        };
        let sequence = self.sequence();
        let connection = Connection::connect(local, remote, sequence, self.mtu, now)?;
        self.streams[slot] = Some(Stream {
            generation,
            owner: Owner::Held,
            connection,
        });
        Ok(StreamId { slot, generation })
    }

    pub fn state(&self, id: StreamId) -> Result<State> {
        Ok(self.stream(id)?.state())
    }

    pub fn endpoints(&self, id: StreamId) -> Result<(Endpoint, Endpoint)> {
        Ok(self.stream(id)?.endpoints())
    }

    pub fn failure(&self, id: StreamId) -> Result<Option<StreamError>> {
        Ok(self.stream(id)?.failure())
    }

    pub fn readiness(&self, id: StreamId) -> Result<Readiness> {
        let connection = self.stream(id)?;
        let state = connection.state();
        Ok(Readiness {
            readable: !connection.receive.is_empty() || connection.peer_closed,
            writable: matches!(state, State::Established | State::CloseWait)
                && !connection.write_closed
                && connection.send.len() < QUEUE_BYTES,
            hangup: state == State::Closed || (connection.peer_closed && connection.write_closed),
            error: connection.failure().is_some(),
        })
    }

    pub fn read(&mut self, id: StreamId, bytes: &mut [u8]) -> Result<usize> {
        self.stream_mut(id)?.read(bytes)
    }

    pub fn send(&mut self, id: StreamId, bytes: &[u8]) -> Result<usize> {
        self.stream_mut(id)?.send(bytes)
    }

    pub fn shutdown_write(&mut self, id: StreamId) -> Result<()> {
        self.stream_mut(id)?.shutdown_write()
    }

    /// Invalidate the owner ID immediately. Once established, the transport
    /// retains a bounded slot for graceful close, FIN retries and TIME_WAIT.
    pub fn close(&mut self, id: StreamId) -> Result<()> {
        let state = self.state(id)?;
        match state {
            State::Closed | State::SynSent | State::SynReceived => {
                self.streams[id.slot] = None;
            }
            State::Established | State::CloseWait => {
                self.stream_mut(id)?.shutdown_write()?;
                self.detach(id);
            }
            State::FinWait1
            | State::FinWait2
            | State::Closing
            | State::LastAck
            | State::TimeWait => {
                self.detach(id);
            }
        }
        Ok(())
    }

    /// Demultiplex a complete TCP segment from validated IPv4 ingress.
    pub fn receive(
        &mut self,
        source: Ip,
        destination: Ip,
        bytes: &[u8],
        now: u64,
    ) -> Result<Ingress> {
        self.advance(now)?;
        if destination != self.local_ip {
            return Err(Error::Address.into());
        }
        if bytes.len() > self.mtu - 20 {
            return Err(Error::Length.into());
        }
        let (source_port, destination_port) = Segment::ports(bytes)?;
        let remote = Endpoint {
            ip: source,
            port: source_port,
        };
        let local = Endpoint {
            ip: destination,
            port: destination_port,
        };
        if let Some(slot) = self.streams.iter_mut().flatten().find(|stream| {
            stream.connection.state() != State::Closed
                && stream.connection.endpoints() == (local, remote)
        }) {
            slot.connection.receive(source, destination, bytes, now)?;
            return Ok(Ingress::Delivered);
        }

        let segment = Segment::parse(remote, local, bytes)?;
        if !segment.has(SYN) || segment.has(ACK | RST | FIN) {
            return Ok(Ingress::Unmatched);
        }
        let Some((slot, listener)) =
            self.listeners
                .iter()
                .enumerate()
                .find_map(|(slot, listener)| {
                    listener
                        .as_ref()
                        .filter(|entry| entry.port == local.port)
                        .map(|entry| (slot, entry))
                })
        else {
            return Ok(Ingress::Unmatched);
        };
        let id = ListenerId {
            slot,
            generation: listener.generation,
        };
        let mut pending = 0;
        let mut backlog = 0;
        for stream in self.streams.iter().flatten() {
            if let Owner::Pending { listener, .. } = stream.owner {
                pending += 1;
                backlog += usize::from(listener == id);
            }
        }
        if backlog >= listener.backlog || pending >= MAX_PENDING {
            return Ok(Ingress::BacklogFull);
        }
        let Ok(slot) = self.free_stream() else {
            return Ok(Ingress::BacklogFull);
        };

        let sequence = self.sequence();
        let connection = Connection::accept(local, remote, sequence, self.mtu, now, bytes)?;
        let generation = Self::generation()?;
        self.streams[slot] = Some(Stream {
            generation,
            owner: Owner::Pending {
                listener: id,
                deadline: now.saturating_add(ADMISSION_MS),
            },
            connection,
        });
        Ok(Ingress::Admitted)
    }

    /// Poll after ingress, stream operations or deadline expiry. Successive
    /// packets rotate across the fixed-size table. A timed-out connection
    /// becomes visible as an error to its owner without stalling its peers.
    pub fn poll(&mut self, now: u64) -> Result<Option<Outbound>> {
        self.poll_egress(now, false)
    }

    /// Reserve a segment for queued egress. Report the handoff with
    /// `complete_egress` before the flight's retransmit timer starts.
    pub fn poll_deferred(&mut self, now: u64) -> Result<Option<Outbound>> {
        self.poll_egress(now, true)
    }

    fn poll_egress(&mut self, now: u64, deferred: bool) -> Result<Option<Outbound>> {
        self.advance(now)?;
        for offset in 0..MAX_STREAMS {
            let index = (self.next_poll + offset) % MAX_STREAMS;
            let Some(stream) = self.streams[index].as_mut() else {
                continue;
            };
            match stream.connection.poll(now) {
                Ok(Some(bytes)) => {
                    let (source, destination) = stream.connection.endpoints();
                    self.next_poll = (index + 1) % MAX_STREAMS;
                    let outgoing = Outbound {
                        source,
                        destination,
                        bytes,
                    };
                    if deferred && !outgoing.ack_only() {
                        if let Some(flight) = &mut stream.connection.flight {
                            flight.awaiting_egress = true;
                            flight.deadline = now.saturating_add(super::EGRESS_WAIT_MS);
                        }
                    }
                    return Ok(Some(outgoing));
                }
                Ok(None) => {}
                Err(error) if stream.connection.failure() != Some(error) => return Err(error),
                Err(_) => {}
            }
            if stream.connection.state() == State::Closed && !matches!(stream.owner, Owner::Held) {
                self.streams[index] = None;
            }
        }
        Ok(None)
    }

    pub fn deadline(&self) -> Option<u64> {
        self.streams
            .iter()
            .flatten()
            .filter_map(|stream| {
                stream
                    .owner_deadline()
                    .into_iter()
                    .chain(stream.connection.deadline())
                    .min()
            })
            .min()
    }

    /// Delayed egress may outlive an acknowledgement or a closed stream.
    pub fn outbound_current(&self, packet: &Outbound) -> bool {
        self.streams.iter().flatten().any(|stream| {
            let connection = &stream.connection;
            if connection.state() == State::Closed
                || connection.endpoints() != (packet.source, packet.destination)
            {
                return false;
            }
            if packet.ack_only() {
                packet.bytes[8..12] == connection.receive_next.to_be_bytes()
            } else {
                connection.flight.is_some_and(|flight| {
                    packet.bytes.get(4..8) == Some(flight.first.to_be_bytes().as_slice())
                        && (flight.kind != super::FlightKind::Syn
                            || (packet.bytes[13] & ACK != 0) == (connection.state() == State::SynReceived))
                })
            }
        })
    }

    /// Start the reserved flight's timer, or fail it if routing has ended
    /// with a terminal error. ACK-only packets have no retransmit timer.
    pub fn complete_egress(
        &mut self,
        packet: &Outbound,
        now: u64,
        handoff: core::result::Result<(), Error>,
    ) -> Result<()> {
        self.advance(now)?;
        if packet.ack_only() || !self.outbound_current(packet) {
            return Ok(());
        }
        let Some(connection) = self.streams.iter_mut().flatten().map(|stream| &mut stream.connection)
            .find(|connection| connection.endpoints() == (packet.source, packet.destination))
        else {
            return Ok(());
        };
        if let Some(flight) = connection.flight.as_mut().filter(|flight| flight.awaiting_egress) {
            if now >= flight.deadline {
                connection.fail(StreamError::Timeout);
                return Ok(());
            }
            match handoff {
                Ok(()) => {
                    flight.awaiting_egress = false;
                    flight.sent_at = now;
                    if flight.persist_delay.is_some() && connection.remote_window != 0 {
                        flight.persist_delay = None;
                        flight.retries = 0;
                    }
                    let delay = flight.persist_delay.unwrap_or(connection.retransmission.timeout);
                    flight.deadline = now.saturating_add(delay);
                }
                Err(error) => connection.fail(error.into()),
            }
        }
        Ok(())
    }

    fn advance(&mut self, now: u64) -> Result<()> {
        if now < self.now {
            return Err(StreamError::Clock);
        }
        self.now = now;
        for slot in &mut self.streams {
            if slot.as_ref().is_some_and(|stream| {
                !matches!(stream.owner, Owner::Held)
                    && (stream.connection.state() == State::Closed
                        || stream
                            .owner_deadline()
                            .is_some_and(|deadline| now >= deadline))
            }) {
                *slot = None;
            }
        }
        Ok(())
    }

    fn detach(&mut self, id: StreamId) {
        self.streams[id.slot].as_mut().unwrap().owner = Owner::Detached {
            deadline: self.now.saturating_add(DETACHED_MS),
        };
    }

    fn listener(&self, id: ListenerId) -> Result<&Listener> {
        self.listeners
            .get(id.slot)
            .and_then(Option::as_ref)
            .filter(|listener| listener.generation == id.generation)
            .ok_or(StreamError::Closed)
    }

    fn held_stream(stream: &Stream, id: StreamId) -> bool {
        stream.generation == id.generation && matches!(stream.owner, Owner::Held)
    }

    fn stream(&self, id: StreamId) -> Result<&Connection> {
        self.streams
            .get(id.slot)
            .and_then(Option::as_ref)
            .filter(|stream| Self::held_stream(stream, id))
            .map(|stream| &stream.connection)
            .ok_or(StreamError::Closed)
    }

    fn stream_mut(&mut self, id: StreamId) -> Result<&mut Connection> {
        self.streams
            .get_mut(id.slot)
            .and_then(Option::as_mut)
            .filter(|stream| Self::held_stream(stream, id))
            .map(|stream| &mut stream.connection)
            .ok_or(StreamError::Closed)
    }

    fn ready_slot(&self, id: ListenerId) -> Option<usize> {
        self.streams
            .iter()
            .enumerate()
            .filter_map(|(slot, stream)| stream.as_ref().map(|stream| (slot, stream)))
            .filter(|(_, stream)| {
                stream.pending_for(id)
                    && matches!(
                        stream.connection.state(),
                        State::Established | State::CloseWait
                    )
            })
            .min_by_key(|(_, stream)| stream.generation)
            .map(|(slot, _)| slot)
    }

    fn free_stream(&self) -> Result<usize> {
        self.streams
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity.into())
    }

    pub fn port_in_use(&self, port: u16) -> bool {
        self.listeners
            .iter()
            .flatten()
            .any(|listener| listener.port == port)
            || self
                .streams
                .iter()
                .flatten()
                .any(|stream| stream.connection.endpoints().0.port == port)
    }

    pub fn ephemeral_port(&mut self) -> Result<u16> {
        // There are far more ephemeral ports than represented local bindings.
        for _ in 0..=MAX_STREAMS + MAX_LISTENERS {
            let port = self.next_port;
            self.next_port = if port == u16::MAX {
                FIRST_EPHEMERAL
            } else {
                port + 1
            };
            if !self.port_in_use(port) {
                return Ok(port);
            }
        }
        Err(Error::Capacity.into())
    }

    fn sequence(&mut self) -> u32 {
        self.sequence_seed ^= self.sequence_seed << 13;
        self.sequence_seed ^= self.sequence_seed >> 17;
        self.sequence_seed ^= self.sequence_seed << 5;
        self.sequence_seed
    }

    fn generation() -> Result<u64> {
        let mut previous = GENERATIONS.load(Ordering::Relaxed);
        loop {
            let next = previous.checked_add(1).ok_or(Error::Capacity)?;
            match GENERATIONS.compare_exchange_weak(
                previous,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Ok(next),
                Err(current) => previous = current,
            }
        }
    }
}

#[cfg(test)]
mod tests;
