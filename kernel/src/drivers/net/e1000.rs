//! E1000 legacy rings shared by the production INTx and private boot paths.
use super::dma::DmaRegion;
use super::rings::{Error as RingError, Rings, COUNT};
#[cfg(any(feature = "boot-smoke-tests", test))]
use super::rings::BUFFER;
use dunit_packet_abi::{MIN_FRAME_LEN, MAX_FRAME_LEN};

const CTRL: usize = 0x0000;
const STATUS: usize = 0x0008;
const MDIC: usize = 0x0020;
const ICR: usize = 0x00c0;
const IMC: usize = 0x00d8;
const IMS: usize = 0x00d0;
const ICS: usize = 0x00c8;
const INTERRUPTS: u32 = 1 | (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7); // TXDW, LSC, RXDMT0, RXO, RXT0
#[cfg(any(feature = "boot-smoke-tests", test))]
pub(super) const FRAME_SIZE: usize = 128;
#[cfg(any(feature = "boot-smoke-tests", test))]
pub(super) const HOST_MAC: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
struct Frame {
    bytes: [u8; MAX_FRAME_LEN],
    length: usize,
}
// PMM reclaim remains exact for the boot diagnostic. This region is
// CPU-owned for its entire lifetime and never published to device registers.
struct RxQueue {
    backing: DmaRegion,
    capacity: usize,
    head: usize,
    len: usize,
}
impl RxQueue {
    fn allocate(capacity: usize) -> Option<Self> {
        Some(Self {
            backing: DmaRegion::allocate(core::mem::size_of::<Frame>() * capacity)?,
            capacity,
            head: 0,
            len: 0,
        })
    }

    fn frame(&self, slot: usize) -> &Frame {
        debug_assert!(slot < self.capacity);
        // Aligned, zero-initialized backing. Frame contains only integers.
        unsafe { &*(self.backing.virtual_address() as *const Frame).add(slot) }
    }

    fn frame_mut(&mut self, slot: usize) -> &mut Frame {
        debug_assert!(slot < self.capacity);
        // The queue is never published to DMA, so the mutable borrow owns
        // the slot until the caller returns.
        unsafe { &mut *(self.backing.virtual_address() as *mut Frame).add(slot) }
    }

    fn consume(&mut self, count: usize) {
        debug_assert!(count <= self.len);
        self.head = (self.head + count) % self.capacity;
        self.len -= count;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Packet,
    #[cfg(any(feature = "boot-smoke-tests", test))]
    ExternalDiagnostic,
    #[cfg(any(feature = "boot-smoke-tests", test))]
    Loopback,
}

impl Mode {
    fn uses_irq(self) -> bool {
        #[cfg(any(feature = "boot-smoke-tests", test))]
        if self == Self::Loopback {
            return false;
        }
        true
    }
}
const RCTL: usize = 0x0100;
const TCTL: usize = 0x0400;
const TIPG: usize = 0x0410;
const RDBAL: usize = 0x2800;
const RDBAH: usize = 0x2804;
const RDLEN: usize = 0x2808;
const RDH: usize = 0x2810;
const RDT: usize = 0x2818;
const TDBAL: usize = 0x3800;
const TDBAH: usize = 0x3804;
const TDLEN: usize = 0x3808;
const TDH: usize = 0x3810;
const TDT: usize = 0x3818;
const RAL: usize = 0x5400;
const RAH: usize = 0x5404;
const RESET: u32 = 1 << 26;
const PHY_LOOPBACK: u16 = 1 << 14;
// QEMU's 82540EM receive-control handler suppresses receive for 1000 ms
// after every RCTL write. This private reference-device diagnostic settles
// before sending its first frame; per-frame deadlines stay at 100 ms.
#[cfg(any(feature = "boot-smoke-tests", test))]
const RX_STARTUP_NS: u64 = 1_010_000_000;
const BUDGET_NS: u64 = 100_000_000;
const POLL_LIMIT: usize = 1_000_000;
#[cfg(any(feature = "boot-smoke-tests", test))]
pub(super) const TEST_PACKETS: usize = COUNT * 3 + 7;

pub(super) trait Registers {
    fn read(&mut self, offset: usize) -> u32;
    fn write(&mut self, offset: usize, value: u32);
    fn bus_master(&mut self, enabled: bool) -> bool;
    fn now_ns(&self) -> u64;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Error {
    Reset,
    NoMemory,
    Registers,
    BusMaster,
    #[cfg(any(feature = "boot-smoke-tests", test))]
    Transmit,
    #[cfg(any(feature = "boot-smoke-tests", test))]
    Receive,
    Timeout,
    #[cfg(any(feature = "boot-smoke-tests", test))]
    Payload,
    Stop,
}
fn reset(io: &mut impl Registers) -> bool {
    io.write(IMC, u32::MAX);
    io.write(RCTL, 0);
    io.write(TCTL, 0);
    let control = io.read(CTRL);
    if control == u32::MAX {
        return false;
    }
    io.write(CTRL, control | RESET);
    let deadline = io.now_ns().saturating_add(BUDGET_NS);
    for _ in 0..POLL_LIMIT {
        let value = io.read(CTRL);
        if value != u32::MAX && value & RESET == 0 {
            io.write(IMC, u32::MAX);
            let _ = io.read(ICR);
            return io.read(STATUS) != u32::MAX;
        }
        if io.now_ns() >= deadline {
            break;
        }
        core::hint::spin_loop();
    }
    false
}
fn mdio(io: &mut impl Registers, command: u32) -> Result<u16, Error> {
    io.write(MDIC, command | (1 << 21)); // reference PHY address 1
    let deadline = io.now_ns().saturating_add(BUDGET_NS);
    for _ in 0..POLL_LIMIT {
        let result = io.read(MDIC);
        if result & (1 << 28) != 0 {
            if result & (1 << 30) != 0 {
                return Err(Error::Registers);
            }
            return Ok(result as u16);
        }
        if io.now_ns() >= deadline {
            break;
        }
        core::hint::spin_loop();
    }
    Err(Error::Timeout)
}
fn read_phy(io: &mut impl Registers) -> Result<u16, Error> {
    mdio(io, 1 << 27) // BMCR register 0, MDIC read
}
fn write_phy(io: &mut impl Registers, value: u16) -> Result<(), Error> {
    mdio(io, (1 << 26) | value as u32)?;
    if read_phy(io)? != value {
        return Err(Error::Registers);
    }
    Ok(())
}
#[cfg(any(feature = "boot-smoke-tests", test))]
pub(super) struct Snapshot {
    pub registers: [(usize, u32); 13],
    pub sequence: usize,
    pub tx_completed: u64,
    pub rx_packets: u64,
    pub rx_dropped: u64,
}
pub(super) struct Controller<R: Registers> {
    io: R,
    rings: Rings,
    armed: bool,
    failed: bool,
    mac: [u8; 6],
    original_phy: u16,
    phy_changed: bool,
    #[cfg(any(feature = "boot-smoke-tests", test))]
    sequence: usize,
    mode: Mode,
    #[cfg(any(feature = "boot-smoke-tests", test))]
    ready_at: u64,
    pending: Option<RxQueue>,
    #[cfg(any(feature = "boot-smoke-tests", test))]
    irq_count: u64,
    rx_overruns: u64,
    tx_errors: u64,
}
impl<R: Registers> Controller<R> {
    #[cfg(any(feature = "boot-smoke-tests", test))]
    pub fn start(io: R) -> Result<Self, Error> {
        Self::start_mode(io, Mode::Loopback)
    }

    #[cfg(any(feature = "boot-smoke-tests", test))]
    pub fn start_external(io: R) -> Result<Self, Error> {
        Self::start_mode(io, Mode::ExternalDiagnostic)
    }

    pub fn start_packet(io: R) -> Result<Self, Error> {
        Self::start_mode(io, Mode::Packet)
    }

    fn start_mode(mut io: R, mode: Mode) -> Result<Self, Error> {
        if !reset(&mut io) {
            return Err(Error::Reset);
        }
        let low = io.read(RAL);
        let high = io.read(RAH);
        let mac = [
            low as u8,
            (low >> 8) as u8,
            (low >> 16) as u8,
            (low >> 24) as u8,
            high as u8,
            (high >> 8) as u8,
        ];
        if high & (1 << 31) == 0 || mac == [0; 6] || mac[0] & 1 != 0 {
            return Err(Error::Registers);
        }
        let original_phy = read_phy(&mut io)?;
        let pending = if mode.uses_irq() {
            let capacity = if mode == Mode::Packet { COUNT } else { 8 };
            Some(RxQueue::allocate(capacity).ok_or(Error::NoMemory)?)
        } else {
            None
        };
        let rings = Rings::allocate().ok_or(Error::NoMemory)?;
        let mut device = Self {
            io,
            rings,
            armed: false,
            failed: false,
            mac,
            original_phy,
            phy_changed: false,
            #[cfg(any(feature = "boot-smoke-tests", test))]
            sequence: 0,
            mode,
            #[cfg(any(feature = "boot-smoke-tests", test))]
            ready_at: 0,
            pending,
            #[cfg(any(feature = "boot-smoke-tests", test))]
            irq_count: 0,
            rx_overruns: 0,
            tx_errors: 0,
        };
        let rx = device.rings.rx_physical();
        let tx = device.rings.tx_physical();
        for (offset, value) in [
            (RDBAL, rx as u32),
            (RDBAH, (rx >> 32) as u32),
            (RDLEN, (COUNT * 16) as u32),
            (RDH, 0),
            (RDT, (COUNT - 1) as u32),
            (TDBAL, tx as u32),
            (TDBAH, (tx >> 32) as u32),
            (TDLEN, (COUNT * 16) as u32),
            (TDH, 0),
            (TDT, 0),
        ] {
            device.io.write(offset, value);
        }
        device.rings.publish();
        device.armed = true;
        if !device.io.bus_master(true) {
            device.stop()?;
            return Err(Error::BusMaster);
        }
        // The reference QEMU device implements PHY BMCR loopback, not RCTL's
        // MAC loopback selector. Disable autonegotiation during the local test.
        device.phy_changed = true;
        let phy = if mode.uses_irq() {
            original_phy & !PHY_LOOPBACK
        } else {
            (original_phy & !(1 << 12)) | PHY_LOOPBACK
        };
        if let Err(error) = write_phy(&mut device.io, phy) {
            device.stop()?;
            return Err(error);
        }
        // PHY-local loopback never emits diagnostic frames onto the wire.
        // Legacy 2 KiB RX buffers, accept broadcast, strip FCS; no jumbo frames.
        let control = device.io.read(CTRL) | (1 << 6); // SLU
        device.io.write(CTRL, control);
        device.io.write(TIPG, 10 | (8 << 10) | (6 << 20));
        device
            .io
            .write(TCTL, 2 | (1 << 3) | (0x10 << 4) | (0x40 << 12));
        device.io.write(RCTL, 2 | (1 << 15) | (1 << 26));
        let _ = device.io.read(STATUS); // flush posted setup writes
        #[cfg(any(feature = "boot-smoke-tests", test))]
        {
            device.ready_at = device.io.now_ns().saturating_add(RX_STARTUP_NS);
            if !mode.uses_irq() {
                for _ in 0..100_000_000usize {
                    if device.io.now_ns() >= device.ready_at {
                        return Ok(device);
                    }
                    core::hint::spin_loop();
                }
                return Err(Error::Timeout);
            }
        }
        Ok(device)
    }

    #[cfg(any(feature = "boot-smoke-tests", test))]
    pub fn ready_at(&self) -> u64 {
        self.ready_at
    }

    pub fn enable_irq(&mut self) {
        let _ = self.io.read(ICR);
        self.resume_irq();
    }

    pub fn mask_irq(&mut self) {
        self.io.write(IMC, u32::MAX);
        let _ = self.io.read(STATUS);
    }

    fn resume_irq(&mut self) {
        self.io.write(IMS, INTERRUPTS);
        // Descriptor completion can arrive while causes are masked. Keep the
        // drain bounded without requiring a new wire edge for pending data.
        if self.rings.rx_ready() {
            self.io.write(ICS, 1 << 7);
        }
        let _ = self.io.read(STATUS);
    }

    /// Drain at most one ring after acknowledging a shared INTx interrupt.
    pub fn handle_irq(&mut self) -> bool {
        let Some(pending) = self.pending.as_mut() else { return false; };
        if !self.armed {
            return false;
        }
        let cause = self.io.read(ICR);
        if cause == u32::MAX {
            self.failed = true;
            return false;
        }
        if cause == 0 {
            return false;
        }
        self.io.write(IMC, u32::MAX);
        #[cfg(any(feature = "boot-smoke-tests", test))]
        {
            self.irq_count += 1;
        }
        if self.failed {
            return true;
        }
        if cause & (1 << 6) != 0 {
            self.rx_overruns += 1;
            self.failed = self.mode != Mode::Packet;
        }
        if self.rings.complete().is_err() {
            self.tx_errors += 1;
            self.failed = true;
        }
        let (minimum, maximum) = if self.mode == Mode::Packet {
            (MIN_FRAME_LEN, MAX_FRAME_LEN)
        } else {
            (0, 128)
        };
        for _ in 0..COUNT {
            let slot = (pending.head + pending.len) % pending.capacity;
            let tail = if pending.len < pending.capacity {
                self.rings.receive(&mut pending.frame_mut(slot).bytes[..maximum])
            } else {
                self.rings.receive(&mut [])
            };
            let Some((tail, result)) = tail else { break; };
            self.io.write(RDT, tail);
            match result {
                Ok(length) if length >= minimum => {
                    pending.frame_mut(slot).length = length;
                    pending.len += 1;
                }
                Ok(_) => {
                    self.rings.counters.rx_dropped += 1;
                    self.failed |= self.mode != Mode::Packet;
                }
                Err(_) => self.failed |= self.mode != Mode::Packet,
            }
        }
        if !self.failed {
            self.resume_irq();
        }
        true
    }

    pub fn mac(&self) -> [u8; 6] {
        self.mac
    }

    pub fn link_up(&mut self) -> bool {
        let status = self.io.read(STATUS);
        status != u32::MAX && status & (1 << 1) != 0
    }

    pub fn failed(&self) -> bool {
        self.failed
    }

    pub fn counters(&self) -> &super::rings::Counters {
        &self.rings.counters
    }

    pub fn rx_drops(&self) -> u64 {
        self.rings.counters.rx_dropped + self.rx_overruns
    }

    pub fn tx_errors(&self) -> u64 {
        self.tx_errors + self.rings.counters.tx_full
    }

    pub fn tx_ready(&self) -> bool {
        self.rings.tx_ready()
    }

    pub fn rx_frame(&self, index: usize) -> Option<&[u8]> {
        let pending = self.pending.as_ref()?;
        if index >= pending.len {
            return None;
        }
        let slot = pending.frame((pending.head + index) % pending.capacity);
        Some(&slot.bytes[..slot.length])
    }

    pub fn consume_rx(&mut self, count: usize) {
        if let Some(pending) = self.pending.as_mut() {
            pending.consume(count);
        }
    }

    pub fn submit_packet(&mut self, frame: &[u8]) -> Result<(), RingError> {
        let tail = self.rings.submit(frame)?;
        self.io.write(TDT, tail);
        Ok(())
    }

    #[cfg(any(feature = "boot-smoke-tests", test))]
    pub fn submit_external(&mut self, sequence: usize) -> Result<(), Error> {
        if self.mode != Mode::ExternalDiagnostic || !self.armed || self.failed { return Err(Error::Stop); }
        let mut packet = [0; FRAME_SIZE];
        packet[..6].copy_from_slice(&HOST_MAC);
        packet[6..12].copy_from_slice(&self.mac);
        packet[12..14].copy_from_slice(&[0x88, 0xb5]);
        for (index, byte) in packet[14..].iter_mut().enumerate() {
            *byte = (sequence as u8).wrapping_mul(37).wrapping_add(index as u8);
        }
        self.sequence = sequence;
        let tail = self.rings.submit(&packet).map_err(|_| Error::Transmit)?;
        self.io.write(TDT, tail);
        Ok(())
    }
    /// Reads CPU-owned queue/counters only. A descriptor can advance this probe
    /// exclusively through handle_irq, so polling cannot conceal missing IRQs.
    #[cfg(any(feature = "boot-smoke-tests", test))]
    pub fn external_progress(&mut self, sequence: usize) -> Result<bool, Error> {
        if self.failed { return Err(Error::Receive); }
        let Some(frame) = self.rx_frame(0) else { return Ok(false); };
        if frame.len() != FRAME_SIZE || frame[..6] != self.mac
            || frame[6..12] != HOST_MAC || frame[12..14] != [0x88, 0xb5]
        { self.failed = true; return Err(Error::Payload); }
        for (index, byte) in frame[14..].iter().enumerate() {
            let sent = (sequence as u8).wrapping_mul(37).wrapping_add(index as u8);
            if *byte != (sent ^ 0xa5) { self.failed = true; return Err(Error::Payload); }
        }
        self.consume_rx(1);
        Ok(true)
    }
    #[cfg(any(feature = "boot-smoke-tests", test))]
    pub fn external_complete(&self) -> Result<bool, Error> {
        if self.failed { return Err(Error::Receive); }
        let c = &self.rings.counters;
        Ok(self.irq_count > 0 && self.rx_frame(0).is_none()
            && c.tx_packets == TEST_PACKETS as u64 && c.tx_completed == TEST_PACKETS as u64
            && c.rx_packets == TEST_PACKETS as u64 && c.rx_dropped == 0
            && c.tx_bytes == (TEST_PACKETS * FRAME_SIZE) as u64
            && c.rx_bytes == (TEST_PACKETS * FRAME_SIZE) as u64)
    }
    #[cfg(any(feature = "boot-smoke-tests", test))]
    pub fn irq_count(&self) -> u64 { self.irq_count }
    #[cfg(any(feature = "boot-smoke-tests", test))]
    pub fn diagnostic(&mut self) -> Result<(), Error> {
        if self.failed || !self.armed {
            return Err(Error::Stop);
        }
        let result = self.run_packets();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    #[cfg(any(feature = "boot-smoke-tests", test))]
    fn run_packets(&mut self) -> Result<(), Error> {
        let mut packet = [0u8; 128];
        let mut received = [0u8; BUFFER];
        packet[..6].copy_from_slice(&self.mac);
        packet[6..12].copy_from_slice(&self.mac);
        packet[12..14].copy_from_slice(&[0x88, 0xb5]); // local experimental EtherType
        for sequence in 0..TEST_PACKETS {
            self.sequence = sequence;
            for (index, byte) in packet[14..].iter_mut().enumerate() {
                *byte = (sequence as u8).wrapping_mul(37).wrapping_add(index as u8);
            }
            let tail = self.rings.transmit(&packet).map_err(|_| Error::Transmit)?;
            self.io.write(TDT, tail);
            let deadline = self.io.now_ns().saturating_add(BUDGET_NS);
            let mut matched = false;
            for _ in 0..POLL_LIMIT {
                if let Some((tail, result)) = self.rings.receive(&mut received) {
                    self.io.write(RDT, tail);
                    let length = result.map_err(|_| Error::Receive)?;
                    if length != packet.len() || received[..length] != packet {
                        return Err(Error::Payload);
                    }
                    matched = true;
                    break;
                }
                if self.io.now_ns() >= deadline {
                    break;
                }
                core::hint::spin_loop();
            }
            if !matched {
                return Err(Error::Timeout);
            }
        }
        let deadline = self.io.now_ns().saturating_add(BUDGET_NS);
        for _ in 0..POLL_LIMIT {
            if self.rings.complete().map_err(|error| match error {
                RingError::Completion => Error::Transmit,
                _ => Error::Receive,
            })? {
                if self.rings.counters.tx_completed != TEST_PACKETS as u64
                    || self.rings.counters.rx_packets != TEST_PACKETS as u64
                    || self.rings.counters.tx_bytes != (TEST_PACKETS * packet.len()) as u64
                    || self.rings.counters.rx_bytes != (TEST_PACKETS * packet.len()) as u64
                {
                    return Err(Error::Payload);
                }
                return Ok(());
            }
            if self.io.now_ns() >= deadline {
                break;
            }
            core::hint::spin_loop();
        }
        Err(Error::Timeout)
    }
    #[cfg(any(feature = "boot-smoke-tests", test))]
    pub fn snapshot(&mut self) -> Snapshot {
        let offsets = [
            CTRL, STATUS, MDIC, RCTL, TCTL, RDH, RDT, TDH, TDT, RDBAL, RDBAH, TDBAL, TDBAH,
        ];
        Snapshot {
            registers: offsets.map(|offset| (offset, self.io.read(offset))),
            sequence: self.sequence,
            tx_completed: self.rings.counters.tx_completed,
            rx_packets: self.rings.counters.rx_packets,
            rx_dropped: self.rings.counters.rx_dropped,
        }
    }
    pub fn stop(&mut self) -> Result<(), Error> {
        if !self.armed {
            return Ok(());
        }
        self.failed = true;
        // Do not free pages merely because RX/TX enable bits cleared. Reset
        // completion and PCI bus-master disable must both be observed.
        let reset_done = reset(&mut self.io);
        let mastering_disabled = self.io.bus_master(false);
        let readback = self.io.read(STATUS);
        let phy_restored = !self.phy_changed || write_phy(&mut self.io, self.original_phy).is_ok();
        if !reset_done || !mastering_disabled || readback == u32::MAX || !phy_restored {
            return Err(Error::Stop);
        }
        unsafe {
            self.rings.retire_after_reset();
        }
        self.armed = false;
        self.phy_changed = false;
        Ok(())
    }
}
impl<R: Registers> Drop for Controller<R> {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
