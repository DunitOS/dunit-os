//! E1000 legacy descriptors. CPU/device handoff is volatile plus x86 DMA fences.
use super::dma::DmaRegion;
use core::sync::atomic::{fence, Ordering};

pub(super) const COUNT: usize = 32;
pub(super) const BUFFER: usize = 2048;
pub(super) const MAX_FRAME: usize = 1518;
const RX_OFFSET: usize = 0;
const TX_OFFSET: usize = 4096;
const BUFFERS_OFFSET: usize = 8192;
const BYTES: usize = BUFFERS_OFFSET + COUNT * BUFFER * 2;
const DD: u8 = 1;
const EOP: u8 = 2;
#[repr(C)]
#[derive(Clone, Copy)]
struct RxDescriptor {
    address: u64,
    length: u16,
    checksum: u16,
    status: u8,
    errors: u8,
    special: u16,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct TxDescriptor {
    address: u64,
    length: u16,
    cso: u8,
    command: u8,
    status: u8,
    css: u8,
    special: u16,
}
const _: () = {
    assert!(core::mem::size_of::<RxDescriptor>() == 16);
    assert!(core::mem::size_of::<TxDescriptor>() == 16);
    assert!(core::mem::offset_of!(RxDescriptor, status) == 12);
    assert!(core::mem::offset_of!(TxDescriptor, status) == 12);
    assert!(COUNT * 16 % 128 == 0);
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Error {
    InvalidLength,
    Full,
    Completion,
    Malformed,
    OutputTooSmall,
}
#[derive(Default)]
pub(super) struct Counters {
    pub tx_packets: u64,
    pub tx_bytes: u64,
    pub tx_completed: u64,
    pub tx_full: u64,
    pub rx_packets: u64,
    pub rx_bytes: u64,
    pub rx_dropped: u64,
}
pub(super) struct Rings {
    dma: DmaRegion,
    tx_next: usize,
    rx_next: usize,
    busy: [bool; COUNT],
    queued: usize,
    discard_chain: bool,
    pub counters: Counters,
}
impl Rings {
    pub fn allocate() -> Option<Self> {
        let dma = DmaRegion::allocate(BYTES)?;
        let rings = Self {
            dma,
            tx_next: 0,
            rx_next: 0,
            busy: [false; COUNT],
            queued: 0,
            discard_chain: false,
            counters: Counters::default(),
        };
        for i in 0..COUNT {
            unsafe {
                core::ptr::write_volatile(
                    rings.rx(i),
                    RxDescriptor {
                        address: rings.dma.physical() as u64 + rings.buffer_offset(false, i) as u64,
                        length: 0,
                        checksum: 0,
                        status: 0,
                        errors: 0,
                        special: 0,
                    },
                );
                core::ptr::write_volatile(
                    rings.tx(i),
                    TxDescriptor {
                        address: rings.dma.physical() as u64 + rings.buffer_offset(true, i) as u64,
                        length: 0,
                        cso: 0,
                        command: 0,
                        status: DD,
                        css: 0,
                        special: 0,
                    },
                );
            }
        }
        Some(rings)
    }
    fn buffer_offset(&self, tx: bool, index: usize) -> usize {
        BUFFERS_OFFSET + (index + if tx { COUNT } else { 0 }) * BUFFER
    }
    fn rx(&self, index: usize) -> *mut RxDescriptor {
        (self.dma.virtual_address() + RX_OFFSET + index * 16) as *mut RxDescriptor
    }
    fn tx(&self, index: usize) -> *mut TxDescriptor {
        (self.dma.virtual_address() + TX_OFFSET + index * 16) as *mut TxDescriptor
    }
    pub fn rx_physical(&self) -> u64 {
        (self.dma.physical() + RX_OFFSET) as u64
    }
    pub fn tx_physical(&self) -> u64 {
        (self.dma.physical() + TX_OFFSET) as u64
    }
    pub fn publish(&mut self) {
        self.dma.publish();
    }
    pub unsafe fn retire_after_reset(&mut self) {
        self.dma.retire_after_reset();
    }
    /// The returned tail must be published to TDT only after this function.
    pub fn transmit(&mut self, frame: &[u8]) -> Result<u32, Error> {
        if !(14..=MAX_FRAME).contains(&frame.len()) {
            return Err(Error::InvalidLength);
        }
        let index = self.tx_next;
        for i in 0..COUNT {
            self.reclaim(i)?;
        }
        if self.queued == COUNT - 1 || self.busy[index] {
            self.counters.tx_full += 1;
            return Err(Error::Full);
        }
        unsafe {
            let buffer = (self.dma.virtual_address() + self.buffer_offset(true, index)) as *mut u8;
            core::ptr::copy_nonoverlapping(frame.as_ptr(), buffer, frame.len());
            core::ptr::write_volatile(
                self.tx(index),
                TxDescriptor {
                    address: self.dma.physical() as u64 + self.buffer_offset(true, index) as u64,
                    length: frame.len() as u16,
                    cso: 0,
                    command: 1 | 2 | 8, // EOP | IFCS | RS
                    status: 0,
                    css: 0,
                    special: 0,
                },
            );
        }
        self.busy[index] = true;
        self.queued += 1;
        self.tx_next = (index + 1) % COUNT;
        self.counters.tx_packets += 1;
        self.counters.tx_bytes += frame.len() as u64;
        fence(Ordering::SeqCst);
        Ok(self.tx_next as u32)
    }
    fn reclaim(&mut self, index: usize) -> Result<(), Error> {
        if !self.busy[index] {
            return Ok(());
        }
        let status =
            unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*self.tx(index)).status)) };
        if status & DD == 0 {
            return Ok(());
        }
        fence(Ordering::Acquire);
        if status & (2 | 4 | 8) != 0 {
            return Err(Error::Completion);
        }
        self.busy[index] = false;
        self.queued -= 1;
        self.counters.tx_completed += 1;
        Ok(())
    }
    pub fn complete(&mut self) -> Result<bool, Error> {
        for i in 0..COUNT {
            self.reclaim(i)?;
        }
        Ok(!self.busy.iter().any(|busy| *busy))
    }
    /// A completed descriptor is recycled on every result. Caller publishes
    /// the returned index to RDT after consumption, even for a dropped frame.
    pub fn receive(&mut self, output: &mut [u8]) -> Option<(u32, Result<usize, Error>)> {
        let index = self.rx_next;
        let descriptor = self.rx(index);
        let status = unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*descriptor).status)) };
        if status & DD == 0 {
            return None;
        }
        fence(Ordering::Acquire);
        let desc = unsafe { core::ptr::read_volatile(descriptor) };
        let length = desc.length as usize;
        let discard = self.discard_chain;
        self.discard_chain = status & EOP == 0;
        let result = if discard
            || status & EOP == 0
            || desc.errors != 0
            || !(14..=MAX_FRAME).contains(&length)
        {
            Err(Error::Malformed)
        } else if length > output.len() {
            Err(Error::OutputTooSmall)
        } else {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    (self.dma.virtual_address() + self.buffer_offset(false, index)) as *const u8,
                    output.as_mut_ptr(),
                    length,
                );
            }
            self.counters.rx_packets += 1;
            self.counters.rx_bytes += length as u64;
            Ok(length)
        };
        if result.is_err() {
            self.counters.rx_dropped += 1;
        }
        unsafe {
            core::ptr::write_volatile(
                descriptor,
                RxDescriptor {
                    address: self.dma.physical() as u64 + self.buffer_offset(false, index) as u64,
                    length: 0,
                    checksum: 0,
                    status: 0,
                    errors: 0,
                    special: 0,
                },
            );
        }
        self.rx_next = (index + 1) % COUNT;
        fence(Ordering::SeqCst);
        Some((index as u32, result))
    }
}
