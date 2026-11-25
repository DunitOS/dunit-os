//! Bounded TCP (IPv4 & IPv6) state for a single four-tuple. The caller owns
//! admission, address configuration and delivery of each complete outbound
//! segment. Multiple segments may be in flight under Reno congestion control.
use crate::{Error, Ip, IpAddr, MTU};
use alloc::{collections::VecDeque, vec::Vec};

mod link;
mod wire;
pub use wire::{DualEndpoint, Endpoint, Segment, ACK, FIN, MAX_DATA, PSH, RST, SYN};

pub const QUEUE_BYTES: usize = 16384;
const _: () = assert!(QUEUE_BYTES.is_power_of_two());
const DEFAULT_MSS: u16 = 536;
const INITIAL_RTO: u64 = 1000;
const MAX_RTO: u64 = 60_000;
const MAX_RETRANSMITS: u8 = 5;
const EGRESS_WAIT_MS: u64 = 10_000;
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
    awaiting_egress: bool,
    /// Split acknowledgements make the remaining sample ambiguous.
    rtt_sample: bool,
}

struct Retransmission {
    estimate: Option<(u64, u64)>,
    timeout: u64,
}

struct OutOfOrder {
    bytes: Vec<Option<u8>>,
    buffered: usize,
    fin: Option<u32>,
}

impl OutOfOrder {
    fn new() -> Self {
        Self { bytes: alloc::vec![None; QUEUE_BYTES], buffered: 0, fin: None }
    }

    fn slot(sequence: u32) -> usize {
        // QUEUE_BYTES divides the TCP sequence space. A receive window
        // spans at most one turn of this ring, including across u32 wrap.
        sequence as usize % QUEUE_BYTES
    }

    fn insert(&mut self, mut sequence: u32, data: &[u8], fin: bool) {
        for &byte in data {
            if self.fin == Some(sequence) {
                return;
            }
            let slot = &mut self.bytes[Self::slot(sequence)];
            if slot.is_none() {
                *slot = Some(byte);
                self.buffered += 1;
            }
            sequence = sequence.wrapping_add(1);
        }
        if fin && self.fin.is_none() && self.bytes[Self::slot(sequence)].is_none() {
            self.fin = Some(sequence);
        }
    }

    fn drain(&mut self, next: &mut u32, receive: &mut VecDeque<u8>) -> bool {
        while receive.len() < QUEUE_BYTES {
            if self.fin == Some(*next) {
                return true;
            }
            let Some(byte) = self.bytes[Self::slot(*next)].take() else { break };
            receive.push_back(byte);
            self.buffered -= 1;
            *next = next.wrapping_add(1);
        }
        false
    }

    fn is_empty(&self) -> bool {
        self.buffered == 0 && self.fin.is_none()
    }
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
    local: DualEndpoint,
    remote: DualEndpoint,
    state: State,
    error: Option<StreamError>,
    now: u64,
    snd_una: u32,
    send_next: u32,
    receive_next: u32,
    remote_window: usize,
    window_update: (u32, u32),
    local_mss: u16,
    remote_mss: u16,
    cwnd: usize,
    ssthresh: usize,
    ca_bytes: usize,
    dup_acks: u8,
    recover: Option<u32>,
    fast_retransmit: bool,
    send: VecDeque<u8>,
    receive: VecDeque<u8>,
    out_of_order: Option<OutOfOrder>,
    flights: VecDeque<Flight>,
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
        Self::connect_ip(local.into(), remote.into(), initial_sequence, mtu, now)
    }

    pub fn connect_ip(
        local: DualEndpoint,
        remote: DualEndpoint,
        initial_sequence: u32,
        mtu: usize,
        now: u64,
    ) -> Result<Self> {
        Self::new_ip(local, remote, initial_sequence, mtu, now, State::SynSent)
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
        Self::accept_ip(local.into(), remote.into(), initial_sequence, mtu, now, syn)
    }

    pub fn accept_ip(
        local: DualEndpoint,
        remote: DualEndpoint,
        initial_sequence: u32,
        mtu: usize,
        now: u64,
        syn: &[u8],
    ) -> Result<Self> {
        let segment = Segment::parse_ip(remote, local, syn)?;
        if !segment.has(SYN) || segment.has(ACK | RST | FIN) {
            return Err(Error::Unsolicited.into());
        }
        let mut connection = Self::new_ip(
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

    fn new_ip(
        local: DualEndpoint,
        remote: DualEndpoint,
        initial_sequence: u32,
        mtu: usize,
        now: u64,
        state: State,
    ) -> Result<Self> {
        local.validate()?;
        remote.validate()?;
        let ip_header = match (local.ip, remote.ip) {
            (IpAddr::V4(_), IpAddr::V4(_)) => 20,
            (IpAddr::V6(_), IpAddr::V6(_)) => 40,
            _ => return Err(Error::Address.into()),
        };
        if local == remote {
            return Err(Error::Address.into());
        }
        // SYN carries an MSS option. Its header still has to fit the path.
        if !(ip_header + 24..=MTU).contains(&mtu) {
            return Err(Error::Length.into());
        }
        let local_mss = (mtu - ip_header - 20) as u16;
        let mss = usize::from(local_mss.min(DEFAULT_MSS));
        Ok(Self {
            local,
            remote,
            state,
            error: None,
            now,
            snd_una: initial_sequence,
            send_next: initial_sequence,
            receive_next: 0,
            remote_window: 0,
            window_update: (0, 0),
            local_mss,
            remote_mss: DEFAULT_MSS,
            cwnd: initial_cwnd(mss),
            ssthresh: u16::MAX as usize,
            ca_bytes: 0,
            dup_acks: 0,
            recover: None,
            fast_retransmit: false,
            send: VecDeque::new(),
            receive: VecDeque::new(),
            out_of_order: None,
            flights: VecDeque::new(),
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

    fn mss(&self) -> usize {
        usize::from(self.local_mss.min(self.remote_mss)).max(1)
    }

    fn data_in_flight(&self) -> usize {
        self.flights
            .iter()
            .filter(|f| f.kind == FlightKind::Data)
            .map(|f| f.length)
            .sum()
    }

    pub fn endpoints(&self) -> (Endpoint, Endpoint) {
        (
            self.local.to_v4().expect("IPv4 local endpoint"),
            self.remote.to_v4().expect("IPv4 remote endpoint"),
        )
    }

    pub const fn endpoints_ip(&self) -> (DualEndpoint, DualEndpoint) {
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
            self.fast_retransmit.then_some(self.now),
        ]
        .into_iter()
        .flatten()
        .chain(self.flights.iter().map(|flight| flight.deadline))
        .min()
    }

    pub(super) fn mark_awaiting_egress(&mut self, bytes: &[u8], deadline: u64) {
        let Some(seq) = bytes.get(4..8).and_then(|s| s.try_into().ok()).map(u32::from_be_bytes) else {
            return;
        };
        if let Some(flight) = self.flights.iter_mut().find(|f| f.first == seq) {
            flight.awaiting_egress = true;
            flight.deadline = deadline;
        }
    }

    pub(super) fn outbound_current(&self, bytes: &[u8], ack_only: bool) -> bool {
        if self.state == State::Closed {
            return false;
        }
        if ack_only {
            return bytes.get(8..12) == Some(self.receive_next.to_be_bytes().as_slice());
        }
        let Some(seq) = bytes.get(4..8).and_then(|s| s.try_into().ok()).map(u32::from_be_bytes) else {
            return false;
        };
        let is_ack = bytes.get(13).is_some_and(|f| f & ACK != 0);
        self.flights.iter().any(|flight| {
            flight.first == seq
                && (flight.kind != FlightKind::Syn || is_ack == (self.state == State::SynReceived))
        })
    }

    pub(super) fn complete_egress(
        &mut self,
        bytes: &[u8],
        now: u64,
        handoff: core::result::Result<(), Error>,
    ) {
        let Some(seq) = bytes.get(4..8).and_then(|s| s.try_into().ok()).map(u32::from_be_bytes) else {
            return;
        };
        let Some(flight) = self
            .flights
            .iter_mut()
            .find(|flight| flight.awaiting_egress && flight.first == seq)
        else {
            return;
        };
        if now >= flight.deadline {
            self.fail(StreamError::Timeout);
            return;
        }
        match handoff {
            Ok(()) => {
                flight.awaiting_egress = false;
                flight.sent_at = now;
                if flight.persist_delay.is_some() && self.remote_window != 0 {
                    flight.persist_delay = None;
                    flight.retries = 0;
                }
                let delay = flight.persist_delay.unwrap_or(self.retransmission.timeout);
                flight.deadline = now.saturating_add(delay);
            }
            Err(error) => self.fail(error.into()),
        }
    }

    /// Supply the verified IP source/destination and the complete TCP payload.
    pub fn receive(&mut self, source: Ip, destination: Ip, bytes: &[u8], now: u64) -> Result<()> {
        self.receive_ip(IpAddr::V4(source), IpAddr::V4(destination), bytes, now)
    }

    pub fn receive_ip(
        &mut self,
        source: IpAddr,
        destination: IpAddr,
        bytes: &[u8],
        now: u64,
    ) -> Result<()> {
        if (source, destination) != (self.remote.ip, self.local.ip) {
            return Err(Error::Address.into());
        }
        let segment = Segment::parse_ip(self.remote, self.local, bytes)?;
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
        self.cwnd = initial_cwnd(self.mss());
    }

    fn in_syn_sent(&mut self, segment: Segment<'_>) -> Result<()> {
        if self.flights.is_empty() {
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
            if let Some(flight) = self.flights.front_mut() {
                flight.awaiting_egress = false;
                flight.deadline = self.now;
            }
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
                if let Some(flight) = self.flights.front_mut() {
                    flight.awaiting_egress = false;
                    flight.deadline = self.now;
                }
                return Ok(());
            }
            if !self.flights.is_empty() && segment.acknowledgement == self.send_next {
                self.acknowledge(segment.acknowledgement)?;
                self.state = State::Established;
                self.remote_window = usize::from(segment.window);
                self.ack_due = true;
                return Ok(());
            }
        }
        if self.flights.is_empty()
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
                && !self.peer_closed
            {
                self.acknowledge(segment.acknowledgement)?;
                self.out_of_order.get_or_insert_with(OutOfOrder::new)
                    .insert(segment.sequence, segment.data, segment.has(FIN));
            }
            return Ok(());
        }
        if overlap > 0 && overlap >= span {
            self.ack_due |= span != 0 || self.in_receive_window(segment.sequence);
            return Ok(());
        }
        // A payload outside the receive window can still be a persist probe
        // with an ACK for our reverse traffic. It never consumes receive space.
        let prev_window = self.remote_window;
        let prev_una = self.snd_una;
        let had_data_flight = self.data_in_flight() > 0;
        let (update, acked_data) = self.acknowledge(segment.acknowledgement)?;
        let is_persist = self
            .flights
            .front()
            .is_some_and(|f| f.persist_delay.is_some());
        if acked_data > 0 {
            self.on_ack_reno(segment.acknowledgement, acked_data);
        } else if update
            && had_data_flight
            && !is_persist
            && prev_window > 0
            && segment.acknowledgement == prev_una
            && span == 0
            && usize::from(segment.window) == prev_window
        {
            self.on_dup_ack_reno();
        }
        if update && offset == 0 && self.accept_window_update(segment) {
            self.remote_window = usize::from(segment.window);
            if self.remote_window != 0 {
                self.persist_at = None;
            }
            if let Some(flight) = self
                .flights
                .front_mut()
                .filter(|flight| flight.persist_delay.is_some())
            {
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
        let data = &segment.data[overlap..];
        if let Some(buffered) = &mut self.out_of_order {
            let available = QUEUE_BYTES - self.receive.len();
            let count = data.len().min(available);
            let fin = segment.has(FIN) && data.len() < available;
            buffered.insert(self.receive_next, &data[..count], fin);
            let fin = buffered.drain(&mut self.receive_next, &mut self.receive);
            if fin || buffered.is_empty() {
                self.out_of_order = None;
            }
            if fin {
                self.deliver(&[], true);
            }
        } else {
            self.deliver(data, segment.has(FIN));
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
    fn acknowledge(&mut self, ack: u32) -> Result<(bool, usize)> {
        if self.flights.is_empty() {
            if ack == self.send_next {
                return Ok((true, 0));
            }
            if earlier(ack, self.send_next) {
                return Ok((false, 0));
            }
            return Err(Error::Unsolicited.into());
        }
        let first = self.snd_una;
        let total = self.send_next.wrapping_sub(first) as usize;
        let count = ack.wrapping_sub(first) as usize;
        if count > total {
            if earlier(ack, first) {
                return Ok((false, 0));
            }
            return Err(Error::Unsolicited.into());
        }
        if count == 0 {
            return Ok((true, 0)); // Duplicate ACK.
        }
        self.snd_una = ack;
        let mut remaining = count;
        let mut data_acked = 0usize;
        while remaining > 0 {
            let mut front = self.flights.pop_front().expect("flight span matches send_next - snd_una");
            if remaining < front.length {
                if front.kind == FlightKind::Data {
                    data_acked += remaining;
                }
                front.first = ack;
                front.length -= remaining;
                // A cumulative ACK advanced the left edge. The outstanding
                // suffix gets a fresh retry budget, while Karn's rule still
                // excludes this flight from RTT sampling.
                front.retries = 0;
                front.rtt_sample = false;
                front.awaiting_egress = false;
                front.deadline = self.now.saturating_add(self.retransmission.timeout);
                self.flights.push_front(front);
                break;
            }
            remaining -= front.length;
            if front.kind == FlightKind::Data {
                data_acked += front.length;
            }
            if front.rtt_sample && !front.awaiting_egress {
                self.retransmission.sample(self.now - front.sent_at);
            }
            match (front.kind, self.state) {
                (FlightKind::Fin, State::FinWait1) => {
                    self.state = State::FinWait2;
                    self.expire_at = Some(self.now.saturating_add(FIN_WAIT_LIMIT));
                }
                (FlightKind::Fin, State::Closing) => self.enter_time_wait(),
                (FlightKind::Fin, State::LastAck) => self.state = State::Closed,
                _ => {}
            }
        }
        if data_acked > 0 {
            self.send.drain(..data_acked);
        }
        Ok((true, data_acked))
    }

    fn on_ack_reno(&mut self, ack: u32, acked: usize) {
        let mss = self.mss();
        if let Some(recover) = self.recover {
            if !earlier(ack, recover) {
                self.cwnd = self.ssthresh.min(self.cwnd).max(mss);
                self.ca_bytes = 0;
                self.dup_acks = 0;
                self.recover = None;
                self.fast_retransmit = false;
            } else {
                self.cwnd = self.cwnd.saturating_sub(acked).saturating_add(mss).min(QUEUE_BYTES * 2);
                self.fast_retransmit = true;
            }
            return;
        }
        self.dup_acks = 0;
        if self.cwnd < self.ssthresh {
            self.cwnd = self.cwnd.saturating_add(acked.min(mss)).min(QUEUE_BYTES * 2);
        } else {
            self.ca_bytes = self.ca_bytes.saturating_add(acked);
            while self.ca_bytes >= self.cwnd {
                self.ca_bytes -= self.cwnd;
                self.cwnd = self.cwnd.saturating_add(mss).min(QUEUE_BYTES * 2);
            }
        }
    }

    fn on_dup_ack_reno(&mut self) {
        let mss = self.mss();
        self.dup_acks = self.dup_acks.saturating_add(1);
        if self.recover.is_none() {
            if self.dup_acks == 3 {
                let flight = self.data_in_flight();
                self.ssthresh = (flight / 2).max(2 * mss);
                self.cwnd = (self.ssthresh + 3 * mss).min(QUEUE_BYTES * 2);
                self.ca_bytes = 0;
                self.recover = Some(self.send_next);
                self.fast_retransmit = true;
            }
        } else {
            self.cwnd = self.cwnd.saturating_add(mss).min(QUEUE_BYTES * 2);
        }
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
        self.flights.clear();
        self.fast_retransmit = false;
        self.recover = None;
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
        if self.fast_retransmit {
            self.fast_retransmit = false;
            if let Some(front) = self.flights.front_mut() {
                front.rtt_sample = false;
                front.deadline = now.saturating_add(self.retransmission.timeout);
                let entry = *front;
                return self.render(Some(entry)).map(Some);
            }
        }
        if let Some(mut flight) = self.flights.front().copied().filter(|f| now >= f.deadline) {
            let reopened = flight.persist_delay.is_some() && self.remote_window != 0;
            if flight.awaiting_egress || (!reopened && flight.retries == MAX_RETRANSMITS) {
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
                if flight.kind == FlightKind::Data {
                    let mss = self.mss();
                    let outstanding = self.data_in_flight();
                    self.ssthresh = (outstanding / 2).max(2 * mss);
                    self.cwnd = mss;
                    self.ca_bytes = 0;
                    self.dup_acks = 0;
                    self.recover = None;
                }
                self.retransmission.backoff();
                flight.retries += 1;
                self.retransmission.timeout
            };
            flight.rtt_sample = false;
            flight.deadline = now.saturating_add(delay);
            *self.flights.front_mut().unwrap() = flight;
            for later in self.flights.iter_mut().skip(1) {
                later.deadline = later.deadline.max(flight.deadline);
            }
            return self.render(Some(flight)).map(Some);
        }
        if matches!(self.state, State::SynSent | State::SynReceived) {
            if self.flights.is_empty() {
                return self.start_flight(FlightKind::Syn, 1).map(Some);
            }
        } else if matches!(self.state, State::Established | State::CloseWait) {
            let in_flight = self.data_in_flight();
            let unsent = self.send.len().saturating_sub(in_flight);
            if unsent > 0 {
                let window = self.remote_window.min(self.cwnd);
                let ready = if self.remote_window != 0 {
                    self.persist_at = None;
                    window.saturating_sub(in_flight)
                } else if in_flight == 0 {
                    let deadline = self
                        .persist_at
                        .get_or_insert_with(|| now.saturating_add(INITIAL_RTO));
                    usize::from(now >= *deadline)
                } else {
                    0
                };
                if ready != 0 {
                    self.persist_at = None;
                    let length = unsent.min(ready).min(self.mss());
                    return self.start_flight(FlightKind::Data, length).map(Some);
                }
            } else if self.write_closed && self.send.is_empty() && self.flights.is_empty() {
                self.persist_at = None;
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
            awaiting_egress: false,
            rtt_sample: persist_delay.is_none(),
        };
        self.send_next = self.send_next.wrapping_add(length as u32);
        self.flights.push_back(flight);
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
                        let offset = entry.first.wrapping_sub(self.snd_una) as usize;
                        length = entry.length;
                        for (to, from) in payload
                            .iter_mut()
                            .zip(self.send.iter().skip(offset))
                            .take(length)
                        {
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
        .encode_ip(self.local, self.remote)?;
        self.ack_due = false;
        Ok(output)
    }
}

fn initial_cwnd(mss: usize) -> usize {
    if mss > 2190 {
        2 * mss
    } else if mss > 1095 {
        (3 * mss).min(4380)
    } else {
        (4 * mss).min(4380)
    }
}

fn earlier(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) < 0
}

#[cfg(test)]
mod tests;

pub mod diagnostic;
pub mod table;
