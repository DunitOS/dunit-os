//! Single owner for the reference NIC's production packet queues.

use super::{e1000, rings, LegacyRoute, MappedRegisters, PciDevice};
use crate::sync::{IrqSafeSpinLock, SpinLock};
use crate::syscall::{EAGAIN, EBUSY, EFBIG, EIO, ENOENT, ENOMEM, EOPNOTSUPP, EPIPE};
use dunit_packet_abi::{encode_record, record_len, Records, MAX_BATCH_LEN, MAX_RECORDS};

pub(crate) const READ: u32 = 1;
pub(crate) const WRITE: u32 = 2;
pub(crate) const HANGUP: u32 = 4;
pub(crate) const ERROR: u32 = 8;

#[derive(Clone, Copy)]
struct Reference {
    device: PciDevice,
    mac: [u8; 6],
    link_up: bool,
}

static REFERENCE: SpinLock<Option<Reference>> = SpinLock::new(None);
static SESSION: IrqSafeSpinLock<Owner> = IrqSafeSpinLock::new(Owner {
    generation: 0,
    state: State::Idle,
});

struct Owner {
    generation: u64,
    state: State,
}

enum State {
    Idle,
    Opening,
    Active(Session),
    Closing,
    Quarantined,
}

struct Session {
    line: u8,
    controller: e1000::Controller<MappedRegisters>,
    route: LegacyRoute,
    rx_reserved: bool,
}

#[derive(Default)]
pub(crate) struct Info {
    pub mac: [u8; 6],
    pub link_up: bool,
    pub rx_packets: u64,
    pub rx_bytes: u64,
    pub rx_dropped: u64,
    pub tx_packets: u64,
    pub tx_bytes: u64,
    pub tx_errors: u64,
    pub next_record: usize,
}

pub(crate) struct Reservation {
    pub bytes: usize,
    count: usize,
    generation: u64,
}

pub(super) fn discover(device: PciDevice, mac: [u8; 6], link_up: bool) {
    *REFERENCE.lock() = Some(Reference { device, mac, link_up });
}

fn interface(index: usize) -> Result<Reference, i64> {
    if index != 0 {
        return Err(EOPNOTSUPP);
    }
    let reference = *REFERENCE.lock();
    let reference = reference.ok_or_else(|| {
        if super::snapshot().total_nics == 0 { ENOENT } else { EOPNOTSUPP }
    })?;
    if !matches!(reference.device.interrupt_line, 10 | 11) {
        return Err(EOPNOTSUPP);
    }
    Ok(reference)
}

pub(crate) fn supports(index: usize) -> Result<(), i64> {
    interface(index).map(|_| ())
}

pub(crate) fn info(index: usize, generation: u64) -> Result<Info, i64> {
    let reference = interface(index)?;
    let mut owner = SESSION.lock();
    let owns_queue = owner.generation == generation;
    match &mut owner.state {
        State::Quarantined => Err(EIO),
        State::Active(session) => {
            let controller = &mut session.controller;
            let link_up = controller.link_up();
            let counters = controller.counters();
            Ok(Info {
                mac: controller.mac(),
                link_up,
                rx_packets: counters.rx_packets,
                rx_bytes: counters.rx_bytes,
                rx_dropped: controller.rx_drops(),
                tx_packets: counters.tx_packets,
                tx_bytes: counters.tx_bytes,
                tx_errors: controller.tx_errors(),
                next_record: if owns_queue {
                    controller.rx_frame(0)
                        .and_then(|frame| record_len(frame.len()).ok())
                        .unwrap_or(0)
                } else {
                    0
                },
            })
        }
        _ => Ok(Info {
            mac: reference.mac,
            link_up: reference.link_up,
            ..Info::default()
        }),
    }
}

pub(crate) fn open(index: usize) -> Result<u64, i64> {
    let reference = interface(index)?;
    let generation = {
        let mut owner = SESSION.lock();
        match owner.state {
            State::Idle => {},
            State::Quarantined => return Err(EIO),
            _ => return Err(EBUSY),
        }
        owner.generation = owner.generation.checked_add(1).ok_or(EIO)?;
        owner.state = State::Opening;
        owner.generation
    };

    // No owner/IRQ guard over MMIO mapping, allocation or reset waits. State
    // Opening excludes another controller until setup or verified rollback.
    let device = reference.device;
    let started = (|| {
        let registers = MappedRegisters::open(device).map_err(|_| e1000::Error::Registers)?;
        super::pci::set_command(device, super::pci::command(device) & !(1 << 10));
        e1000::Controller::start_packet(registers)
    })();

    let mut owner = SESSION.lock();
    match started {
        Ok(mut controller) => {
            let line = device.interrupt_line;
            let route = LegacyRoute::open(line);
            controller.enable_irq();
            owner.state = State::Active(Session { line, controller, route, rx_reserved: false });
            Ok(generation)
        }
        Err(error) => {
            owner.state = match error {
                e1000::Error::Reset | e1000::Error::Stop => State::Quarantined,
                _ => State::Idle,
            };
            Err(if error == e1000::Error::NoMemory { ENOMEM } else { EIO })
        }
    }
}

pub(crate) fn close(generation: u64) {
    let _ = stop(generation);
}

/// Reset revokes the current generation. A later open creates fresh DMA rings.
pub(crate) fn reset(index: usize) -> Result<(), i64> {
    supports(index)?;
    let generation = {
        let owner = SESSION.lock();
        match owner.state {
            State::Active(_) => owner.generation,
            State::Quarantined => return Err(EIO),
            State::Idle => return Err(EPIPE),
            _ => return Err(EBUSY),
        }
    };
    stop(generation)
}

fn stop(generation: u64) -> Result<(), i64> {
    let mut session = {
        let mut owner = SESSION.lock();
        if generation != owner.generation || !matches!(owner.state, State::Active(_)) {
            return Err(EPIPE);
        }
        let State::Active(mut session) = core::mem::replace(&mut owner.state, State::Closing) else {
            unreachable!();
        };
        // Exclude an in-flight IRQ drain and deassert the hardware causes
        // before making the controller unreachable from the IRQ path.
        session.controller.mask_irq();
        session
    };
    crate::process::wake_poll_waiters();
    let result = session.controller.stop();
    let Session { controller, route, .. } = session;
    drop(controller);
    drop(route);
    SESSION.lock().state = if result.is_ok() { State::Idle } else { State::Quarantined };
    result.map_err(|_| EIO)
}

impl Owner {
    fn active(&mut self, generation: u64) -> Result<&mut Session, i64> {
        if self.generation == generation {
            match &mut self.state {
                State::Active(session) => return Ok(session),
                State::Quarantined => return Err(EIO),
                State::Idle | State::Opening | State::Closing => {}
            }
        }
        Err(EPIPE)
    }
}

impl Session {
    fn ready(&self) -> u32 {
        if self.controller.failed() {
            return ERROR | HANGUP;
        }
        let read = !self.rx_reserved && self.controller.rx_frame(0).is_some();
        let write = self.controller.tx_ready();
        (if read { READ } else { 0 }) | (if write { WRITE } else { 0 })
    }
}

pub(crate) fn ready(generation: u64) -> u32 {
    match SESSION.lock().active(generation) {
        Ok(session) => session.ready(),
        Err(EPIPE) => HANGUP,
        Err(_) => ERROR | HANGUP,
    }
}

pub(super) fn handle_irq(line: u8) -> bool {
    let (handled, notify) = {
        let mut owner = SESSION.lock();
        let State::Active(session) = &mut owner.state else { return false; };
        if session.line != line {
            return false;
        }
        let before = session.ready();
        let handled = session.controller.handle_irq();
        (handled, session.ready() & !before != 0)
    };
    if notify {
        crate::process::wake_poll_waiters();
    }
    handled
}

pub(crate) fn transmit(generation: u64, records: Records<'_>) -> Result<usize, i64> {
    let mut owner = SESSION.lock();
    let session = owner.active(generation)?;
    if session.controller.failed() {
        return Err(EIO);
    }
    let mut accepted = 0;
    for frame in records {
        // Records validated all frame lengths and padding before entering the
        // owner scope. Full is the only normal ring submission failure.
        let size = record_len(frame.len()).map_err(|_| EIO)?;
        match session.controller.submit_packet(frame) {
            Ok(()) => accepted += size,
            Err(rings::Error::Full) => break,
            Err(_) => return Err(EIO),
        }
    }
    if accepted == 0 { Err(EAGAIN) } else { Ok(accepted) }
}

/// Reserve a complete front prefix into a kernel-owned buffer. No caller
/// mapping is touched under the controller lock. IRQ appends cannot overwrite
/// the reserved slots and a concurrent reader observes no READ readiness.
pub(crate) fn reserve(generation: u64, bytes: &mut [u8]) -> Result<Reservation, i64> {
    let mut owner = SESSION.lock();
    let session = owner.active(generation)?;
    if session.controller.failed() {
        return Err(EIO);
    }
    if session.rx_reserved {
        return Err(EAGAIN);
    }
    let len = bytes.len().min(MAX_BATCH_LEN);
    let bytes = &mut bytes[..len];
    let mut reservation = Reservation { bytes: 0, count: 0, generation };
    for index in 0..MAX_RECORDS {
        let Some(frame) = session.controller.rx_frame(index) else { break; };
        let size = record_len(frame.len()).map_err(|_| EIO)?;
        if size > bytes.len() - reservation.bytes {
            if reservation.count == 0 {
                return Err(EFBIG);
            }
            break;
        }
        encode_record(frame, &mut bytes[reservation.bytes..]).map_err(|_| EIO)?;
        reservation.bytes += size;
        reservation.count += 1;
    }
    if reservation.count == 0 {
        return Err(EAGAIN);
    }
    session.rx_reserved = true;
    Ok(reservation)
}

pub(crate) fn finish(generation: u64, reservation: Reservation, consume: bool) -> Result<(), i64> {
    if reservation.generation != generation {
        return Err(EPIPE);
    }
    let (notify, result) = {
        let mut owner = SESSION.lock();
        let session = owner.active(generation)?;
        debug_assert!(session.rx_reserved);
        session.rx_reserved = false;
        if session.controller.failed() {
            (false, Err(EIO))
        } else {
            if consume {
                session.controller.consume_rx(reservation.count);
            }
            (session.controller.rx_frame(0).is_some(), Ok(()))
        }
    };
    if notify {
        crate::process::wake_poll_waiters();
    }
    result
}
