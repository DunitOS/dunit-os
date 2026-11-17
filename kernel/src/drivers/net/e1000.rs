//! Private E1000 legacy-ring bring-up. Polling is only a boot diagnostic;
//! production packet queues and interrupt delivery need their own contracts.
use super::rings::{Error as RingError, Rings, BUFFER, COUNT};

const CTRL: usize = 0x0000;
const STATUS: usize = 0x0008;
const MDIC: usize = 0x0020;
const ICR: usize = 0x00c0;
const IMC: usize = 0x00d8;
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
const RX_STARTUP_NS: u64 = 1_010_000_000;
const BUDGET_NS: u64 = 100_000_000;
const POLL_LIMIT: usize = 1_000_000;
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
    Transmit,
    Receive,
    Timeout,
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
    sequence: usize,
}
impl<R: Registers> Controller<R> {
    pub fn start(mut io: R) -> Result<Self, Error> {
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
        let rings = Rings::allocate().ok_or(Error::NoMemory)?;
        let mut device = Self {
            io,
            rings,
            armed: false,
            failed: false,
            mac,
            original_phy,
            phy_changed: false,
            sequence: 0,
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
            return Err(Error::BusMaster);
        }
        // The reference QEMU device implements PHY BMCR loopback, not RCTL's
        // MAC loopback selector. Disable autonegotiation during the local test.
        device.phy_changed = true;
        write_phy(&mut device.io, (original_phy & !(1 << 12)) | PHY_LOOPBACK)?;
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
        let ready_at = device.io.now_ns().saturating_add(RX_STARTUP_NS);
        let mut ready = false;
        for _ in 0..100_000_000usize {
            if device.io.now_ns() >= ready_at {
                ready = true;
                break;
            }
            core::hint::spin_loop();
        }
        if !ready {
            return Err(Error::Timeout);
        }
        Ok(device)
    }
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
