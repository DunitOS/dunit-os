//! Bounded TCPv4 state for a single four-tuple. The caller owns admission,
//! address configuration and delivery of each complete outbound segment.
//! One segment is in flight. The receiver can retain one out-of-order segment.
use crate::{Error, Ip, MTU};
use alloc::{collections::VecDeque, vec::Vec};

mod link;
mod wire;
pub use wire::{Endpoint, Segment, ACK, FIN, MAX_DATA, PSH, RST, SYN};

pub const QUEUE_BYTES: usize = 4096;
const DEFAULT_MSS: u16 = 536;
const INITIAL_RTO: u64 = 1000;
const MAX_RTO: u64 = 60_000;
const MAX_RETRANSMITS: u8 = 5;
const TIME_WAIT: u64 = 60_000;
const FIN_WAIT_LIMIT: u64 = 60_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Closed,
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    LastAck,
    TimeWait,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamError {
    Packet(Error),
    Closed,
    Reset,
    WouldBlock,
    Timeout,
    Clock,
}

impl From<Error> for StreamError {
    fn from(error: Error) -> Self {
        Self::Packet(error)
    }
}

type Result<T> = core::result::Result<T, StreamError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FlightKind {
    Syn,
    Data,
    Fin,
}

#[derive(Clone, Copy, Debug)]
struct Flight {
    kind: FlightKind,
    first: u32,
    length: usize,
    sent_at: u64,
    deadline: u64,
    retries: u8,
    persist_delay: Option<u64>,
    /// Split acknowledgements make the remaining sample ambiguous.
    rtt_sample: bool,
}

struct Retransmission {
    estimate: Option<(u64, u64)>,
    timeout: u64,
}

struct OutOfOrder {
    sequence: u32,
    data: Vec<u8>,
    fin: bool,
}

impl Retransmission {
    fn sample(&mut self, ms: u64) {
        let ms = ms.clamp(1, MAX_RTO);
        let (mean, variance) = match self.estimate {
            Some((mean, variance)) => ((7 * mean + ms) / 8, (3 * variance + mean.abs_diff(ms)) / 4),
            None => (ms, ms / 2),
        };
        self.estimate = Some((mean, variance));
        self.timeout = (mean + (4 * variance).max(1)).clamp(INITIAL_RTO, MAX_RTO);
    }

    fn backoff(&mut self) {
        self.timeout = (self.timeout * 2).min(MAX_RTO);
    }
}

pub struct Connection {
    local: Endpoint,
    remote: Endpoint,
    state: State,
    error: Option<StreamError>,
    now: u64,
    send_next: u32,
    receive_next: u32,
    remote_window: usize,
    window_update: (u32, u32),
    local_mss: u16,
    remote_mss: u16,
    send: VecDeque<u8>,
    receive: VecDeque<u8>,
    out_of_order: Option<OutOfOrder>,
    flight: Option<Flight>,
    retransmission: Retransmission,
    ack_due: bool,
    write_closed: bool,
    peer_closed: bool,
    expire_at: Option<u64>,
    persist_at: Option<u64>,
}

impl Connection {
    pub fn connect(
        local: Endpoint,
        remote: Endpoint,
        initial_sequence: u32,
        mtu: usize,
        now: u64,
    ) -> Result<Self> {
        Self::new(local, remote, initial_sequence, mtu, now, State::SynSent)
    }

    /// Validate the first SYN before admitting the remote to a connection.
    /// A listening caller owns the number of concurrent admissions.
    pub fn accept(
        local: Endpoint,
        remote: Endpoint,
        initial_sequence: u32,
        mtu: usize,
        now: u64,
        syn: &[u8],
    ) -> Result<Self> {
        let segment = Segment::parse(remote, local, syn)?;
        if !segment.has(SYN) || segment.has(ACK | RST | FIN) {
            return Err(Error::Unsolicited.into());
        }
        let mut connection = Self::new(
            local,
            remote,
            initial_sequence,
            mtu,
            now,
            State::SynReceived,
        )?;
        connection.receive_syn(segment);
        Ok(connection)
    }

    fn new(
        local: Endpoint,
        remote: Endpoint,
        initial_sequence: u32,
        mtu: usize,
        now: u64,
        state: State,
    ) -> Result<Self> {
        local.validate()?;
        remote.validate()?;
        if local == remote {
            return Err(Error::Address.into());
        }
        // SYN carries an MSS option. Its header still has to fit the path.
        if !(44..=MTU).contains(&mtu) {
            return Err(Error::Length.into());
        }
        Ok(Self {
            local,
            remote,
            state,
            error: None,
            now,
            send_next: initial_sequence,
            receive_next: 0,
            remote_window: 0,
            window_update: (0, 0),
            local_mss: (mtu - 40) as u16,
            remote_mss: DEFAULT_MSS,
            send: VecDeque::new(),
            receive: VecDeque::new(),
            out_of_order: None,
            flight: None,
            retransmission: Retransmission {
                estimate: None,
                timeout: INITIAL_RTO,
            },
            ack_due: false,
            write_closed: false,
            peer_closed: false,
            expire_at: None,
            persist_at: None,
        })
    }

    pub const fn endpoints(&self) -> (Endpoint, Endpoint) {
        (self.local, self.remote)
    }

    pub const fn state(&self) -> State {
        self.state
    }

    pub const fn failure(&self) -> Option<StreamError> {
        self.error
    }

    pub fn send(&mut self, bytes: &[u8]) -> Result<usize> {
        self.check_failure()?;
        if self.write_closed || !matches!(self.state, State::Established | State::CloseWait) {
            return Err(StreamError::Closed);
        }
        let count = bytes.len().min(QUEUE_BYTES - self.send.len());
        if count == 0 && !bytes.is_empty() {
            return Err(StreamError::WouldBlock);
        }
        self.send.extend(&bytes[..count]);
        Ok(count)
    }

    pub fn read(&mut self, bytes: &mut [u8]) -> Result<usize> {
        self.check_failure()?;
        let count = bytes.len().min(self.receive.len());
        if count == 0 && !bytes.is_empty() && !self.peer_closed {
            return Err(StreamError::WouldBlock);
        }
        for (output, input) in bytes.iter_mut().zip(self.receive.drain(..count)) {
            *output = input;
        }
        if count != 0 && !self.peer_closed {
            self.ack_due = true; // Advertise newly opened space.
        }
        Ok(count)
    }

    /// Drain already accepted writes before transmitting FIN.
    pub fn shutdown_write(&mut self) -> Result<()> {
        self.check_failure()?;
        if !matches!(self.state, State::Established | State::CloseWait) {
            return Err(StreamError::Closed);
        }
        self.write_closed = true;
        Ok(())
    }

    /// Absolute deadline for retransmission, persist probes or close expiry.
    /// Poll immediately after enqueueing bytes, reading, shutdown or ingress.
    pub fn deadline(&self) -> Option<u64> {
        [
            self.expire_at,
            self.persist_at,
            self.flight.map(|flight| flight.deadline),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Supply the verified IP source/destination and the complete TCP payload.
    pub fn receive(&mut self, source: Ip, destination: Ip, bytes: &[u8], now: u64) -> Result<()> {
        if (source, destination) != (self.remote.ip, self.local.ip) {
            return Err(Error::Address.into());
        }
        let segment = Segment::parse(self.remote, self.local, bytes)?;
        self.advance(now)?;
        match self.state {
            State::Closed => Err(StreamError::Closed),
            State::SynSent => self.in_syn_sent(segment),
            State::SynReceived => self.in_syn_received(segment),
            State::TimeWait => {
                let end = segment.sequence.wrapping_add(segment.data.len() as u32 + 1);
                if segment.has(FIN) && segment.has(ACK) && end == self.receive_next {
                    self.ack_due = true;
                    self.expire_at = Some(self.now.saturating_add(TIME_WAIT));
                }
                Ok(())
            }
            _ => self.in_synchronized(segment),
        }
    }

    fn receive_syn(&mut self, segment: Segment<'_>) {
        self.receive_next = segment.sequence.wrapping_add(1);
        self.remote_mss = segment.mss.unwrap_or(DEFAULT_MSS);
        self.remote_window = usize::from(segment.window);
        self.window_update = (segment.sequence, segment.acknowledgement);
    }

    fn in_syn_sent(&mut self, segment: Segment<'_>) -> Result<()> {
        if self.flight.is_none() {
            return Err(Error::Unsolicited.into());
        }
        let acknowledges_syn = segment.has(ACK) && segment.acknowledgement == self.send_next;
        if segment.has(ACK) && !acknowledges_syn {
            return Err(Error::Unsolicited.into());
        }
        if segment.has(RST) {
            if acknowledges_syn {
                self.fail(StreamError::Reset);
            }
            return Ok(());
        }
        if !segment.has(SYN) {
            return Err(Error::Unsolicited.into());
        }
        self.receive_syn(segment);
        if acknowledges_syn {
            self.acknowledge(segment.acknowledgement)?;
            self.state = State::Established;
            self.ack_due = true;
        } else {
            self.state = State::SynReceived; // Simultaneous open.
            self.flight.as_mut().unwrap().deadline = self.now;
        }
        Ok(())
    }

    fn in_syn_received(&mut self, segment: Segment<'_>) -> Result<()> {
        if segment.has(RST) && segment.sequence == self.receive_next {
            self.fail(StreamError::Reset);
            return Ok(());
        }
        if segment.has(SYN) && segment.sequence == self.receive_next.wrapping_sub(1) {
            if !segment.has(ACK) {
                // An already emitted SYN-ACK still has a bounded retry budget.
                if let Some(flight) = &mut self.flight {
                    flight.deadline = self.now;
                }
                return Ok(());
            }
            if self.flight.is_some() && segment.acknowledgement == self.send_next {
                self.acknowledge(segment.acknowledgement)?;
                self.state = State::Established;
                self.remote_window = usize::from(segment.window);
                self.ack_due = true;
                return Ok(());
            }
        }
        if self.flight.is_none()
            || segment.sequence != self.receive_next
            || !segment.has(ACK)
            || segment.acknowledgement != self.send_next
            || segment.has(SYN | RST)
        {
            return Err(Error::Unsolicited.into());
        }
        self.acknowledge(segment.acknowledgement)?;
        self.state = State::Established;
        self.in_synchronized(segment)
    }

    fn in_synchronized(&mut self, segment: Segment<'_>) -> Result<()> {
        if segment.has(RST) {
            if segment.sequence == self.receive_next {
                self.fail(StreamError::Reset);
            } else if self.in_receive_window(segment.sequence) {
                self.ack_due = true; // Challenge a reset with a non-exact sequence.
            }
            return Ok(());
        }
        if segment.has(SYN) || !segment.has(ACK) {
            self.ack_due = true;
            return Ok(());
        }
        let offset = segment.sequence.wrapping_sub(self.receive_next) as i32;
        let overlap = if offset < 0 {
            offset.unsigned_abs() as usize
        } else {
            0
        };
        let span = segment.data.len() + usize::from(segment.has(FIN));
        if offset > 0 {
            self.ack_due |= span != 0 || self.in_receive_window(segment.sequence);
            let available = QUEUE_BYTES - self.receive.len();
            if span != 0
                && offset as usize + span <= available
                && self.out_of_order.is_none()
                && !self.peer_closed
            {
                self.acknowledge(segment.acknowledgement)?;
                self.out_of_order = Some(OutOfOrder {
                    sequence: segment.sequence,
                    data: segment.data.to_vec(),
                    fin: segment.has(FIN),
                });
            }
            return Ok(());
        }
        if overlap > 0 && overlap >= span {
            self.ack_due |= span != 0 || self.in_receive_window(segment.sequence);
            return Ok(());
        }
        // A payload outside the receive window can still be a persist probe
        // with an ACK for our reverse traffic. It never consumes receive space.
        let update = self.acknowledge(segment.acknowledgement)?;
        if update && offset == 0 && self.accept_window_update(segment) {
            self.remote_window = usize::from(segment.window);
            if self.remote_window != 0 {
                self.persist_at = None;
            }
            if let Some(flight) = self.flight.as_mut().filter(|flight| flight.persist_delay.is_some()) {
                if self.remote_window == 0 {
                    // The peer still answers. Keep probing with backoff,
                    // without charging successful probes as losses.
                    flight.retries = 0;
                } else {
                    // Retry the outstanding probe immediately now that the
                    // receiver can accept it.
                    flight.deadline = self.now;
                }
            }
        }
        if span == 0 {
            return Ok(());
        }
        self.ack_due = true;
        if self.peer_closed {
            return Ok(());
        }
        self.receive_payload(segment, overlap);
        Ok(())
    }

    fn receive_payload(&mut self, segment: Segment<'_>, overlap: usize) {
        if let Some(buffered) = &self.out_of_order {
            let gap = buffered.sequence.wrapping_sub(self.receive_next) as usize;
            if segment.data.len().saturating_sub(overlap) >= gap {
                // Fill the gap, then deliver the first copy of overlapping
                // bytes. The saved segment already fits within this window.
                self.deliver(&segment.data[overlap..overlap + gap], false);
                let buffered = self.out_of_order.take().unwrap();
                self.deliver(&buffered.data, buffered.fin);
            }
        }
        let overlap = self.receive_next.wrapping_sub(segment.sequence) as usize;
        if !self.peer_closed && overlap <= segment.data.len() {
            self.deliver(&segment.data[overlap..], segment.has(FIN));
        }
    }

    fn deliver(&mut self, data: &[u8], fin: bool) {
        let available = QUEUE_BYTES - self.receive.len();
        let count = data.len().min(available);
        self.receive.extend(&data[..count]);
        self.receive_next = self.receive_next.wrapping_add(count as u32);
        if fin && count == data.len() && count < available {
            self.receive_next = self.receive_next.wrapping_add(1);
            self.peer_closed = true;
            self.out_of_order = None;
            match self.state {
                State::Established => self.state = State::CloseWait,
                State::FinWait1 => self.state = State::Closing,
                State::FinWait2 => self.enter_time_wait(),
                _ => {}
            }
        }
    }

    fn in_receive_window(&self, sequence: u32) -> bool {
        (sequence.wrapping_sub(self.receive_next) as usize) < QUEUE_BYTES - self.receive.len()
    }

    fn accept_window_update(&mut self, segment: Segment<'_>) -> bool {
        let (sequence, acknowledgement) = self.window_update;
        if earlier(sequence, segment.sequence)
            || (segment.sequence == sequence && !earlier(segment.acknowledgement, acknowledgement))
        {
            self.window_update = (segment.sequence, segment.acknowledgement);
            return true;
        }
        false
    }

    /// Whether this ACK is current enough to update the sender window.
    fn acknowledge(&mut self, ack: u32) -> Result<bool> {
        let first = self.flight.map_or(self.send_next, |flight| flight.first);
        let Some(mut flight) = self.flight else {
            if ack == self.send_next {
                return Ok(true);
            }
            if earlier(ack, self.send_next) {
                return Ok(false);
            }
            return Err(Error::Unsolicited.into());
        };
        let count = ack.wrapping_sub(first) as usize;
        if count > flight.length {
            if earlier(ack, first) {
                return Ok(false);
            }
            return Err(Error::Unsolicited.into());
        }
        if count == 0 {
            return Ok(true); // Duplicate ACK.
        }
        if flight.kind == FlightKind::Data {
            self.send.drain(..count);
            self.remote_window = self.remote_window.saturating_sub(count);
        }
        if count != flight.length {
            flight.first = ack;
            flight.length -= count;
            flight.rtt_sample = false;
            flight.deadline = self.now.saturating_add(self.retransmission.timeout);
            self.flight = Some(flight);
            return Ok(true);
        }
        self.flight = None;
        if flight.rtt_sample {
            self.retransmission.sample(self.now - flight.sent_at);
        }
        match (flight.kind, self.state) {
            (FlightKind::Fin, State::FinWait1) => {
                self.state = State::FinWait2;
                self.expire_at = Some(self.now.saturating_add(FIN_WAIT_LIMIT));
            }
            (FlightKind::Fin, State::Closing) => self.enter_time_wait(),
            (FlightKind::Fin, State::LastAck) => self.state = State::Closed,
            _ => {}
        }
        Ok(true)
    }

    fn enter_time_wait(&mut self) {
        self.state = State::TimeWait;
        self.expire_at = Some(self.now.saturating_add(TIME_WAIT));
    }

    fn check_failure(&self) -> Result<()> {
        self.error.map_or(Ok(()), Err)
    }

    fn fail(&mut self, error: StreamError) {
        self.state = State::Closed;
        self.error = Some(error);
        self.send.clear();
        self.receive.clear();
        self.out_of_order = None;
        self.flight = None;
        self.expire_at = None;
        self.persist_at = None;
        self.ack_due = false;
    }

    fn advance(&mut self, now: u64) -> Result<()> {
        if now < self.now {
            return Err(StreamError::Clock);
        }
        self.now = now;
        if self.expire_at.is_some_and(|deadline| now >= deadline) {
            if self.state == State::TimeWait {
                self.state = State::Closed;
                self.expire_at = None;
            } else {
                self.fail(StreamError::Timeout);
            }
        }
        self.check_failure()
    }

    pub fn poll(&mut self, now: u64) -> Result<Option<Vec<u8>>> {
        self.advance(now)?;
        if self.state == State::Closed {
            return Ok(None);
        }
        if let Some(mut flight) = self.flight {
            if now >= flight.deadline {
                let reopened = flight.persist_delay.is_some() && self.remote_window != 0;
                if !reopened && flight.retries == MAX_RETRANSMITS {
                    self.fail(StreamError::Timeout);
                    return Err(StreamError::Timeout);
                }
                let delay = if reopened {
                    flight.persist_delay = None;
                    flight.retries = 0;
                    self.retransmission.timeout
                } else if let Some(delay) = &mut flight.persist_delay {
                    *delay = (*delay * 2).min(MAX_RTO);
                    flight.retries += 1;
                    *delay
                } else {
                    self.retransmission.backoff();
                    flight.retries += 1;
                    self.retransmission.timeout
                };
                flight.rtt_sample = false;
                flight.deadline = now.saturating_add(delay);
                self.flight = Some(flight);
                return self.render(Some(flight)).map(Some);
            }
        } else if matches!(self.state, State::SynSent | State::SynReceived) {
            return self.start_flight(FlightKind::Syn, 1).map(Some);
        } else if matches!(self.state, State::Established | State::CloseWait)
            && (!self.send.is_empty() || self.write_closed)
        {
            let ready = if self.remote_window != 0 {
                self.persist_at = None;
                self.remote_window
            } else {
                let deadline = self
                    .persist_at
                    .get_or_insert_with(|| now.saturating_add(INITIAL_RTO));
                if now >= *deadline {
                    1
                } else {
                    0
                }
            };
            if ready != 0 {
                self.persist_at = None;
                if !self.send.is_empty() {
                    let length = self
                        .send
                        .len()
                        .min(ready)
                        .min(usize::from(self.local_mss.min(self.remote_mss)));
                    return self.start_flight(FlightKind::Data, length).map(Some);
                }
                self.state = if self.state == State::Established {
                    State::FinWait1
                } else {
                    State::LastAck
                };
                return self.start_flight(FlightKind::Fin, 1).map(Some);
            }
        }
        if self.ack_due && !matches!(self.state, State::SynSent | State::SynReceived) {
            return self.render(None).map(Some);
        }
        Ok(None)
    }

    fn start_flight(&mut self, kind: FlightKind, length: usize) -> Result<Vec<u8>> {
        let persist_delay = (kind != FlightKind::Syn && self.remote_window == 0)
            .then_some(INITIAL_RTO);
        let flight = Flight {
            kind,
            first: self.send_next,
            length,
            sent_at: self.now,
            deadline: self.now.saturating_add(persist_delay.unwrap_or(self.retransmission.timeout)),
            retries: 0,
            persist_delay,
            rtt_sample: persist_delay.is_none(),
        };
        self.send_next = self.send_next.wrapping_add(length as u32);
        self.flight = Some(flight);
        self.render(Some(flight))
    }

    fn render(&mut self, flight: Option<Flight>) -> Result<Vec<u8>> {
        let mut payload = [0; MAX_DATA];
        let mut length = 0;
        let mut flags = if self.state == State::SynSent { 0 } else { ACK };
        let mut mss = None;
        let sequence = match flight {
            Some(entry) => {
                match entry.kind {
                    FlightKind::Syn => {
                        flags |= SYN;
                        mss = Some(self.local_mss);
                    }
                    FlightKind::Fin => flags |= FIN,
                    FlightKind::Data => {
                        length = entry.length;
                        for (to, from) in payload.iter_mut().zip(self.send.iter()).take(length) {
                            *to = *from;
                        }
                        flags |= PSH;
                    }
                }
                entry.first
            }
            None => self.send_next,
        };
        let output = Segment {
            sequence,
            acknowledgement: if flags & ACK == 0 {
                0
            } else {
                self.receive_next
            },
            flags,
            window: (QUEUE_BYTES - self.receive.len()) as u16,
            mss,
            data: &payload[..length],
        }
        .encode(self.local, self.remote)?;
        self.ack_due = false;
        Ok(output)
    }
}

fn earlier(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) < 0
}

#[cfg(test)]
mod tests;

pub mod diagnostic;
pub mod table;
