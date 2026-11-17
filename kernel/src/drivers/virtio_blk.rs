use crate::drivers::block::{self, BlockDeviceInfo, BlockError};
use crate::drivers::pci::{self, PciBar};
use crate::hal;
use crate::memory::pmm::{get_pmm, PhysicalAddress};
use crate::memory::vmm;
use crate::sync::SpinLock;
use core::sync::atomic::{fence, AtomicBool, Ordering};

const VIRTIO_VENDOR_ID: u16 = 0x1AF4;
const VIRTIO_BLK_LEGACY_DEVICE_ID: u16 = 0x1001;
const VD0_NAME: &str = "vd0";
const SECTOR_SIZE: usize = 512;
const MAX_QUEUE_PAGES: usize = 16;
const VRING_DESC_SIZE: usize = 16;
const VRING_DESC_F_NEXT: u16 = 1;
const VRING_DESC_F_WRITE: u16 = 2;
const VIRTIO_BLK_T_IN: u32 = 0;
const VIRTIO_BLK_T_OUT: u32 = 1;
const VIRTIO_BLK_F_RO: u32 = 1 << 5;

const REG_DEVICE_FEATURES: u16 = 0;
const REG_GUEST_FEATURES: u16 = 4;
const REG_QUEUE_PFN: u16 = 8;
const REG_QUEUE_SIZE: u16 = 12;
const REG_QUEUE_SELECT: u16 = 14;
const REG_QUEUE_NOTIFY: u16 = 16;
const REG_DEVICE_STATUS: u16 = 18;
const REG_CONFIG: u16 = 20;

const STATUS_ACKNOWLEDGE: u8 = 1;
const STATUS_DRIVER: u8 = 2;
const STATUS_DRIVER_OK: u8 = 4;
const STATUS_FAILED: u8 = 128;
const TIMEOUT_SPINS: usize = if cfg!(test) { 64 } else { 10_000_000 };

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct VirtioBlkReq {
    request_type: u32,
    reserved: u32,
    sector: u64,
}

struct VirtioBlkDevice {
    io_base: u16,
    capacity_sectors: u64,
    readonly: bool,
    queue_virt: usize,
    queue_size: usize,
    io_phys: usize,
    io_virt: usize,
    avail_idx: u16,
    used_idx: u16,
    failed: bool,
}

// Polling keeps IRQs enabled; no interrupt handler takes this lock.
static VD0: SpinLock<Option<VirtioBlkDevice>> = SpinLock::new(None);
static STARTED: AtomicBool = AtomicBool::new(false);

pub fn init() {
    if STARTED.swap(true, Ordering::AcqRel) { return; }
    let Some(device) = find_legacy_virtio_blk() else {
        crate::serial_write("[VIRTIO-BLK] no legacy virtio-blk device\r\n");
        return;
    };

    match init_device(device) {
        Some(vd0) => {
            let capacity = vd0.capacity_sectors;
            let readonly = vd0.readonly;
            *VD0.lock() = Some(vd0);
            block::register_device(
                BlockDeviceInfo {
                    name: VD0_NAME,
                    driver: "virtio-blk",
                    block_size: SECTOR_SIZE,
                    blocks: capacity,
                    readonly,
                },
                vd0_read,
                vd0_write,
            );
            crate::serial_write("[VIRTIO-BLK] registered vd0 sectors=");
            write_dec(capacity);
            crate::serial_write(if readonly { " mode=ro\r\n" } else { " mode=rw\r\n" });
        }
        None => crate::serial_write("[VIRTIO-BLK] init failed\r\n"),
    }
}

fn find_legacy_virtio_blk() -> Option<pci::PciDevice> {
    let mut found = None;
    pci::for_each_device(|dev| {
        if found.is_none()
            && dev.vendor_id == VIRTIO_VENDOR_ID
            && dev.device_id == VIRTIO_BLK_LEGACY_DEVICE_ID
        {
            found = Some(dev);
        }
    });
    found
}

fn init_device(dev: pci::PciDevice) -> Option<VirtioBlkDevice> {
    let io_base = match pci::read_bar_decoded(dev, 0) {
        PciBar::Io(port) => port as u16,
        _ => return None,
    };
    pci::enable_io_bus_master(dev);

    if !reset_device(io_base) { return None; }
    outb(io_base + REG_DEVICE_STATUS, STATUS_ACKNOWLEDGE);
    outb(
        io_base + REG_DEVICE_STATUS,
        STATUS_ACKNOWLEDGE | STATUS_DRIVER,
    );

    let features = inl(io_base + REG_DEVICE_FEATURES) & VIRTIO_BLK_F_RO;
    let readonly = features & VIRTIO_BLK_F_RO != 0;
    outl(io_base + REG_GUEST_FEATURES, features);

    outw(io_base + REG_QUEUE_SELECT, 0);
    // Tear down any queue the firmware left programmed before sizing our own.
    outl(io_base + REG_QUEUE_PFN, 0);
    let queue_size = inw(io_base + REG_QUEUE_SIZE) as usize;
    let queue_bytes = vring_bytes(queue_size);
    if queue_size < 3 || !queue_size.is_power_of_two() || queue_bytes > MAX_QUEUE_PAGES * 4096 {
        crate::serial_write("[VIRTIO-BLK] unsupported queue size=");
        write_dec(queue_size as u64);
        crate::serial_write(" bytes=");
        write_dec(queue_bytes as u64);
        crate::serial_write("\r\n");
        fail_device(io_base);
        return None;
    }

    let capacity_low = inl(io_base + REG_CONFIG) as u64;
    let capacity_high = inl(io_base + REG_CONFIG + 4) as u64;
    let capacity = (capacity_high << 32) | capacity_low;
    if capacity == 0 {
        fail_device(io_base);
        return None;
    }

    let (queue_phys, queue_virt) = match alloc_contiguous_queue(queue_bytes) {
        Some(queue) => queue,
        None => {
            fail_device(io_base);
            return None;
        }
    };
    unsafe {
        core::ptr::write_bytes(queue_virt as *mut u8, 0, queue_bytes);
    }
    let (io_phys, io_virt) = match alloc_frame_pair() {
        Some(frame) => frame,
        None => {
            free_queue(queue_phys, (queue_bytes + 4095) / 4096);
            fail_device(io_base);
            return None;
        }
    };
    unsafe {
        core::ptr::write_bytes(io_virt as *mut u8, 0, 4096);
    }
    outl(io_base + REG_QUEUE_PFN, (queue_phys / 4096) as u32);

    outb(
        io_base + REG_DEVICE_STATUS,
        STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK,
    );

    Some(VirtioBlkDevice {
        io_base,
        capacity_sectors: capacity,
        readonly,
        queue_virt,
        queue_size,
        io_phys,
        io_virt,
        avail_idx: 0,
        used_idx: 0,
        failed: false,
    })
}

fn vd0_read(lba: u64, buf: &mut [u8]) -> Result<usize, BlockError> {
    if buf.is_empty() || buf.len() % SECTOR_SIZE != 0 {
        return Err(BlockError::BufferTooSmall);
    }
    let sectors = buf.len() / SECTOR_SIZE;
    validate_range(lba, sectors)?;
    for i in 0..sectors {
        let mut tmp = [0u8; SECTOR_SIZE];
        transfer(lba + i as u64, &mut tmp, false)?;
        buf[i * SECTOR_SIZE..(i + 1) * SECTOR_SIZE].copy_from_slice(&tmp);
    }
    Ok(buf.len())
}

fn vd0_write(lba: u64, buf: &[u8]) -> Result<usize, BlockError> {
    if buf.is_empty() || buf.len() % SECTOR_SIZE != 0 {
        return Err(BlockError::BufferTooSmall);
    }
    let sectors = buf.len() / SECTOR_SIZE;
    validate_range(lba, sectors)?;
    for i in 0..sectors {
        let mut tmp = [0u8; SECTOR_SIZE];
        tmp.copy_from_slice(&buf[i * SECTOR_SIZE..(i + 1) * SECTOR_SIZE]);
        transfer(lba + i as u64, &mut tmp, true)?;
    }
    Ok(buf.len())
}

fn validate_range(lba: u64, sectors: usize) -> Result<(), BlockError> {
    let guard = VD0.lock();
    let dev = guard.as_ref().ok_or(BlockError::NotFound)?;
    if dev.failed { return Err(BlockError::Io); }
    let end = lba.checked_add(sectors as u64).ok_or(BlockError::OutOfRange)?;
    if end > dev.capacity_sectors { return Err(BlockError::OutOfRange); }
    Ok(())
}

fn transfer(lba: u64, buf: &mut [u8], write: bool) -> Result<usize, BlockError> {
    if buf.len() < SECTOR_SIZE {
        return Err(BlockError::BufferTooSmall);
    }

    let mut guard = VD0.lock();
    let dev = guard.as_mut().ok_or(BlockError::NotFound)?;
    if dev.failed { return Err(BlockError::Io); }
    if write && dev.readonly { return Err(BlockError::Io); }
    if lba >= dev.capacity_sectors {
        return Err(BlockError::OutOfRange);
    }

    unsafe {
        let header_ptr = dev.io_virt as *mut VirtioBlkReq;
        (*header_ptr).request_type = if write {
            VIRTIO_BLK_T_OUT
        } else {
            VIRTIO_BLK_T_IN
        };
        (*header_ptr).reserved = 0;
        (*header_ptr).sector = lba;

        let data_virt = dev.io_virt + 64;
        let status_virt = dev.io_virt + 64 + SECTOR_SIZE;
        let status_ptr = status_virt as *mut u8;
        core::ptr::write_volatile(status_ptr, 0xFF);
        if write {
            core::ptr::copy_nonoverlapping(buf.as_ptr(), data_virt as *mut u8, SECTOR_SIZE);
        } else {
            core::ptr::write_bytes(data_virt as *mut u8, 0, SECTOR_SIZE);
        }

        write_desc(
            dev.queue_virt,
            0,
            dev.io_phys as u64,
            core::mem::size_of::<VirtioBlkReq>() as u32,
            VRING_DESC_F_NEXT,
            1,
        );
        write_desc(
            dev.queue_virt,
            1,
            (dev.io_phys + 64) as u64,
            SECTOR_SIZE as u32,
            if write {
                VRING_DESC_F_NEXT
            } else {
                VRING_DESC_F_NEXT | VRING_DESC_F_WRITE
            },
            2,
        );
        write_desc(
            dev.queue_virt,
            2,
            (dev.io_phys + 64 + SECTOR_SIZE) as u64,
            1,
            VRING_DESC_F_WRITE,
            0,
        );

        let avail = avail_offset(dev.queue_virt, dev.queue_size);
        let ring_slot = dev.avail_idx as usize % dev.queue_size;
        write_u16(avail + 4 + ring_slot * 2, 0);
        dev.avail_idx = dev.avail_idx.wrapping_add(1);
        fence(Ordering::Release);
        write_u16(avail + 2, dev.avail_idx);

        fence(Ordering::SeqCst);
        outw(dev.io_base + REG_QUEUE_NOTIFY, 0);

        let used = used_offset(dev.queue_virt, dev.queue_size);
        let mut spins = 0usize;
        while read_u16(used + 2) == dev.used_idx && spins < TIMEOUT_SPINS {
            core::hint::spin_loop();
            spins += 1;
        }
        if read_u16(used + 2) == dev.used_idx {
            offline(dev);
            return Err(BlockError::Io);
        }
        fence(Ordering::Acquire);
        let used_id = core::ptr::read_volatile(
            (used + 4 + (dev.used_idx as usize % dev.queue_size) * 8) as *const u32);
        if read_u16(used + 2) != dev.used_idx.wrapping_add(1) || used_id != 0 {
            offline(dev);
            return Err(BlockError::Io);
        }
        dev.used_idx = dev.used_idx.wrapping_add(1);

        if core::ptr::read_volatile(status_ptr) != 0 {
            return Err(BlockError::Io);
        }
        if !write {
            core::ptr::copy_nonoverlapping(data_virt as *const u8, buf.as_mut_ptr(), SECTOR_SIZE);
        }
    }

    Ok(SECTOR_SIZE)
}

fn write_desc(base: usize, index: usize, addr: u64, len: u32, flags: u16, next: u16) {
    let ptr = base + index * VRING_DESC_SIZE;
    unsafe {
        write_u64(ptr, addr);
        write_u32(ptr + 8, len);
        write_u16(ptr + 12, flags);
        write_u16(ptr + 14, next);
    }
}

fn avail_offset(base: usize, queue_size: usize) -> usize {
    base + queue_size * VRING_DESC_SIZE
}

fn used_offset(base: usize, queue_size: usize) -> usize {
    (base + queue_size * VRING_DESC_SIZE + 4 + queue_size * 2 + 2 + 4095) & !4095
}

fn vring_bytes(queue_size: usize) -> usize {
    let used = used_offset(0, queue_size);
    used + 4 + queue_size * 8 + 2
}

fn alloc_contiguous_queue(bytes: usize) -> Option<(usize, usize)> {
    let pmm = get_pmm()?;
    let pages = (bytes + 4095) / 4096;
    if pages == 0 || pages > MAX_QUEUE_PAGES {
        return None;
    }

    let mut phys = pmm.alloc_contiguous(pages)?.as_usize();
    if phys == 0 {
        // PFN 0 disables the queue. Keep the first run owned while asking for
        // another, then return it; no allocator retry or frame-zero leak.
        let next = pmm.alloc_contiguous(pages);
        free_queue(phys, pages);
        phys = next?.as_usize();
    }
    if phys / 4096 > u32::MAX as usize {
        free_queue(phys, pages);
        return None;
    }
    Some((phys, vmm::phys_to_virt(phys)))
}

fn alloc_frame_pair() -> Option<(usize, usize)> {
    let pmm = get_pmm()?;
    let phys = pmm.alloc_frame()?.as_usize();
    Some((phys, vmm::phys_to_virt(phys)))
}

fn free_queue(phys: usize, pages: usize) {
    if let Some(pmm) = get_pmm() {
        for page in 0..pages {
            pmm.free_frame(PhysicalAddress::from_usize(phys + page * 4096));
        }
    }
}

fn reset_device(io_base: u16) -> bool {
    outb(io_base + REG_DEVICE_STATUS, 0);
    for _ in 0..TIMEOUT_SPINS {
        if inb(io_base + REG_DEVICE_STATUS) == 0 { return true; }
        core::hint::spin_loop();
    }
    false
}

fn offline(dev: &mut VirtioBlkDevice) {
    dev.failed = true;
    let stopped = reset_device(dev.io_base);
    // Keep queue, header, status and data allocated even on failed reset. No
    // later transfer may edit them while a device could still own the chain.
    crate::serial_write(if stopped {
        "[VIRTIO-BLK] disk offline; reset complete; DMA retained\r\n"
    } else {
        "[VIRTIO-BLK] disk offline; reset failed; DMA quarantined\r\n"
    });
}

fn fail_device(io_base: u16) {
    let status = inb(io_base + REG_DEVICE_STATUS);
    outb(io_base + REG_DEVICE_STATUS, status | STATUS_FAILED);
}

fn inw(port: u16) -> u16 {
    unsafe { hal::hal_inw(port) }
}

fn inb(port: u16) -> u8 {
    unsafe { hal::hal_inb(port) }
}

fn inl(port: u16) -> u32 {
    unsafe { hal::hal_inl(port) }
}

fn outb(port: u16, value: u8) {
    unsafe { hal::hal_outb(port, value) }
}

fn outw(port: u16, value: u16) {
    unsafe { hal::hal_outw(port, value) }
}

fn outl(port: u16, value: u32) {
    unsafe { hal::hal_outl(port, value) }
}

unsafe fn read_u16(addr: usize) -> u16 {
    core::ptr::read_volatile(addr as *const u16)
}

unsafe fn write_u16(addr: usize, value: u16) {
    core::ptr::write_volatile(addr as *mut u16, value);
}

unsafe fn write_u32(addr: usize, value: u32) {
    core::ptr::write_volatile(addr as *mut u32, value);
}

unsafe fn write_u64(addr: usize, value: u64) {
    core::ptr::write_volatile(addr as *mut u64, value);
}

use crate::serial::write_dec;
