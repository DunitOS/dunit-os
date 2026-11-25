//! Multi-NIC owner for reference E1000 packet queues (slots 0..MAX_NICS).

use super::{e1000, rings, LegacyRoute, MappedRegisters, PciDevice};
use crate::sync::{IrqSafeSpinLock, SpinLock};
use crate::syscall::{EAGAIN, EBUSY, EFBIG, EIO, ENOENT, ENOMEM, EOPNOTSUPP, EPIPE};
use dunit_packet_abi::{encode_record, record_len, Records, MAX_BATCH_LEN, MAX_RECORDS};

pub const MAX_NICS: usize = 4;

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

static REFERENCES: SpinLock<[Option<Reference>; MAX_NICS]> = SpinLock::new([None; MAX_NICS]);
static SESSIONS: IrqSafeSpinLock<[Owner; MAX_NICS]> = IrqSafeSpinLock::new([
    Owner::idle(),
    Owner::idle(),
    Owner::idle(),
    Owner::idle(),
]);

struct Owner {
    generation: u64,
    state: State,
}

impl Owner {
    const fn idle() -> Self {
        Self {
            generation: 0,
            state: State::Idle,
        }
    }

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

fn slot_of(generation: u64) -> Option<usize> {
    let tag = (generation >> 56) as usize;
    tag.checked_sub(1).filter(|&idx| idx < MAX_NICS)
}

pub(super) fn discover(index: usize, device: PciDevice, mac: [u8; 6], link_up: bool) {
    if index < MAX_NICS {
        REFERENCES.lock()[index] = Some(Reference { device, mac, link_up });
    }
}

fn interface(index: usize) -> Result<Reference, i64> {
    if index >= MAX_NICS {
        return Err(EOPNOTSUPP);
    }
    let reference = REFERENCES.lock()[index].ok_or_else(|| {
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
    let mut owners = SESSIONS.lock();
    let owner = &mut owners[index];
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
                    controller
                        .rx_frame(0)
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
        let mut owners = SESSIONS.lock();
        let owner = &mut owners[index];
        match owner.state {
            State::Idle => {}
            State::Quarantined => return Err(EIO),
            _ => return Err(EBUSY),
        }
        let seq = (owner.generation & ((1u64 << 56) - 1)).checked_add(1).ok_or(EIO)?;
        let generation = (((index as u64) + 1) << 56) | seq;
        owner.generation = generation;
        owner.state = State::Opening;
        generation
    };

    let device = reference.device;
    let started = (|| {
        let registers = MappedRegisters::open(device).map_err(|_| e1000::Error::Registers)?;
        super::pci::set_command(device, super::pci::command(device) & !(1 << 10));
        e1000::Controller::start_packet(registers)
    })();

    let mut owners = SESSIONS.lock();
    let owner = &mut owners[index];
    match started {
        Ok(mut controller) => {
            let line = device.interrupt_line;
            let route = LegacyRoute::open(line);
            controller.enable_irq();
            owner.state = State::Active(Session {
                line,
                controller,
                route,
                rx_reserved: false,
            });
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

pub(crate) fn reset(index: usize) -> Result<(), i64> {
    supports(index)?;
    let generation = {
        let owners = SESSIONS.lock();
        let owner = &owners[index];
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
    let slot = slot_of(generation).ok_or(EPIPE)?;
    let mut session = {
        let mut owners = SESSIONS.lock();
        let owner = &mut owners[slot];
        if generation != owner.generation || !matches!(owner.state, State::Active(_)) {
            return Err(EPIPE);
        }
        let State::Active(mut session) = core::mem::replace(&mut owner.state, State::Closing) else {
            unreachable!();
        };
        session.controller.mask_irq();
        session
    };
    crate::process::wake_poll_waiters();
    let result = session.controller.stop();
    let Session { controller, route, .. } = session;
    drop(controller);
    drop(route);
    SESSIONS.lock()[slot].state = if result.is_ok() {
        State::Idle
    } else {
        State::Quarantined
    };
    result.map_err(|_| EIO)
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
    let Some(slot) = slot_of(generation) else {
        return HANGUP;
    };
    match SESSIONS.lock()[slot].active(generation) {
        Ok(session) => session.ready(),
        Err(EPIPE) => HANGUP,
        Err(_) => ERROR | HANGUP,
    }
}

pub(super) fn handle_irq(line: u8) -> bool {
    let (any_handled, any_notify) = {
        let mut owners = SESSIONS.lock();
        let mut any_handled = false;
        let mut any_notify = false;
        for owner in owners.iter_mut() {
            let State::Active(session) = &mut owner.state else {
                continue;
            };
            if session.line != line {
                continue;
            }
            let before = session.ready();
            if session.controller.handle_irq() {
                any_handled = true;
            }
            if session.ready() & !before != 0 {
                any_notify = true;
            }
        }
        (any_handled, any_notify)
    };
    if any_notify {
        crate::process::wake_poll_waiters();
    }
    any_handled
}

pub(crate) fn transmit(generation: u64, records: Records<'_>) -> Result<usize, i64> {
    let slot = slot_of(generation).ok_or(EPIPE)?;
    let mut owners = SESSIONS.lock();
    let session = owners[slot].active(generation)?;
    if session.controller.failed() {
        return Err(EIO);
    }
    let mut accepted = 0;
    for frame in records {
        let size = record_len(frame.len()).map_err(|_| EIO)?;
        match session.controller.submit_packet(frame) {
            Ok(()) => accepted += size,
            Err(rings::Error::Full) => break,
            Err(_) => return Err(EIO),
        }
    }
    if accepted == 0 { Err(EAGAIN) } else { Ok(accepted) }
}

pub(crate) fn reserve(generation: u64, bytes: &mut [u8]) -> Result<Reservation, i64> {
    let slot = slot_of(generation).ok_or(EPIPE)?;
    let mut owners = SESSIONS.lock();
    let session = owners[slot].active(generation)?;
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
        let Some(frame) = session.controller.rx_frame(index) else {
            break;
        };
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
    let slot = slot_of(generation).ok_or(EPIPE)?;
    let (notify, result) = {
        let mut owners = SESSIONS.lock();
        let session = owners[slot].active(generation)?;
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
