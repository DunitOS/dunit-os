//! Legacy PCI VirtIO-RNG entropy source (`0x1AF4:0x1005` / `0x1044` I/O BAR0).
use crate::drivers::pci::{self, PciBar, PciDevice};
use crate::hal;
use crate::memory::pmm::{get_pmm, PhysicalAddress};
use crate::memory::vmm;
use crate::sync::IrqSafeSpinLock;
use core::sync::atomic::{compiler_fence, fence, Ordering};

const VIRTIO_VENDOR_ID: u16 = 0x1AF4;
const VIRTIO_RNG_LEGACY_ID: u16 = 0x1005;
const VIRTIO_RNG_MODERN_ID: u16 = 0x1044;

const REG_GUEST_FEATURES: u16 = 0x04;
const REG_QUEUE_PFN: u16 = 0x08;
const REG_QUEUE_SIZE: u16 = 0x0C;
const REG_QUEUE_SELECT: u16 = 0x0E;
const REG_QUEUE_NOTIFY: u16 = 0x10;
const REG_DEVICE_STATUS: u16 = 0x12;

const STATUS_ACKNOWLEDGE: u8 = 0x01;
const STATUS_DRIVER: u8 = 0x02;
const STATUS_DRIVER_OK: u8 = 0x04;
const STATUS_FAILED: u8 = 0x80;

const VRING_DESC_F_WRITE: u16 = 2;
const VRING_DESC_SIZE: usize = 16;
const MAX_QUEUE_PAGES: usize = 8;
const MAX_CHUNK_BYTES: usize = 256;
const TIMEOUT_SPINS: usize = 2_000_000;

struct VirtioRngDevice {
    io_base: u16,
    queue_virt: usize,
    queue_size: usize,
    buf_phys: usize,
    buf_virt: usize,
    avail_idx: u16,
    used_idx: u16,
    failed: bool,
}

static RNG0: IrqSafeSpinLock<Option<VirtioRngDevice>> = IrqSafeSpinLock::new(None);

/// Probe and initialize legacy PCI `virtio-rng` (BAR0 I/O space), then seed ChaCha20 CSPRNG.
pub fn init() {
    let mut candidate: Option<(PciDevice, u16)> = None;
    pci::for_each_device(|dev| {
        if candidate.is_some()
            || dev.vendor_id != VIRTIO_VENDOR_ID
            || (dev.device_id != VIRTIO_RNG_LEGACY_ID && dev.device_id != VIRTIO_RNG_MODERN_ID)
        {
            return;
        }
        if let PciBar::Io(base) = pci::read_bar_decoded(dev, 0) {
            if let Ok(port) = u16::try_from(base) {
                if port != 0 {
                    candidate = Some((dev, port));
                }
            }
        }
    });

    let Some((dev, io_base)) = candidate else {
        return;
    };

    pci::enable_io_bus_master(dev);
    let Some(device) = init_device(io_base) else {
        crate::serial_write("[VIRTIO-RNG] init failed; device marked failed\r\n");
        return;
    };

    let queue_size = device.queue_size;
    *RNG0.lock() = Some(device);

    let mut initial = [0u8; 64];
    let seeded = try_fill_bytes(&mut initial);
    if seeded > 0 {
        crate::entropy::mix_hardware_bytes(&initial[..seeded]);
    }
    zeroize(&mut initial);

    let mut line = crate::FmtBuf::new();
    line.push_str("[VIRTIO-RNG] ready queue_size=");
    line.push_u64(queue_size as u64);
    line.push_str(" seeded=");
    line.push_u64(seeded as u64);
    line.push_str("B\r\n");
    crate::serial_write(line.as_str());
}

/// Pull up to `out.len()` entropy bytes from `virtio-rng-pci` if attached and healthy.
pub fn try_fill_bytes(out: &mut [u8]) -> usize {
    if out.is_empty() {
        return 0;
    }
    let mut guard = RNG0.lock();
    let Some(dev) = guard.as_mut() else {
        return 0;
    };
    if dev.failed {
        return 0;
    }

    let mut offset = 0usize;
    while offset < out.len() {
        let req = (out.len() - offset).min(MAX_CHUNK_BYTES);
        let got = read_chunk(dev, &mut out[offset..offset + req]);
        if got == 0 {
            break;
        }
        offset += got;
    }
    offset
}

fn init_device(io_base: u16) -> Option<VirtioRngDevice> {
    if !reset_device(io_base) {
        return None;
    }
    outb(io_base + REG_DEVICE_STATUS, STATUS_ACKNOWLEDGE);
    outb(io_base + REG_DEVICE_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER);
    outl(io_base + REG_GUEST_FEATURES, 0);
    outw(io_base + REG_QUEUE_SELECT, 0);

    let queue_size = inw(io_base + REG_QUEUE_SIZE) as usize;
    if queue_size == 0 || !queue_size.is_power_of_two() || queue_size > 256 {
        fail_device(io_base);
        return None;
    }

    let queue_bytes = vring_bytes(queue_size);
    let (queue_phys, queue_virt) = match alloc_contiguous_queue(queue_bytes) {
        Some(q) => q,
        None => {
            fail_device(io_base);
            return None;
        }
    };
    unsafe {
        core::ptr::write_bytes(queue_virt as *mut u8, 0, queue_bytes);
    }

    let pmm = match get_pmm() {
        Some(p) => p,
        None => {
            free_queue(queue_phys, (queue_bytes + 4095) / 4096);
            fail_device(io_base);
            return None;
        }
    };
    let Some(buf_frame) = pmm.alloc_frame() else {
        free_queue(queue_phys, (queue_bytes + 4095) / 4096);
        fail_device(io_base);
        return None;
    };
    let buf_phys = buf_frame.as_usize();
    let buf_virt = vmm::phys_to_virt(buf_phys);
    unsafe {
        core::ptr::write_bytes(buf_virt as *mut u8, 0, 4096);
    }

    outl(io_base + REG_QUEUE_PFN, (queue_phys / 4096) as u32);
    outb(
        io_base + REG_DEVICE_STATUS,
        STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK,
    );

    Some(VirtioRngDevice {
        io_base,
        queue_virt,
        queue_size,
        buf_phys,
        buf_virt,
        avail_idx: 0,
        used_idx: 0,
        failed: false,
    })
}

fn read_chunk(dev: &mut VirtioRngDevice, out: &mut [u8]) -> usize {
    let req_len = out.len().min(MAX_CHUNK_BYTES);
    if req_len == 0 {
        return 0;
    }

    unsafe {
        core::ptr::write_bytes(dev.buf_virt as *mut u8, 0, req_len);
        write_desc(
            dev.queue_virt,
            0,
            dev.buf_phys as u64,
            req_len as u32,
            VRING_DESC_F_WRITE,
            0,
        );

        let avail = avail_offset(dev.queue_virt, dev.queue_size);
        let ring_slot = (dev.avail_idx as usize) % dev.queue_size;
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
            return 0;
        }
        fence(Ordering::Acquire);
        let elem_addr = used + 4 + ((dev.used_idx as usize) % dev.queue_size) * 8;
        let used_id = core::ptr::read_volatile(elem_addr as *const u32);
        let used_len = core::ptr::read_volatile((elem_addr + 4) as *const u32);
        if read_u16(used + 2) != dev.used_idx.wrapping_add(1) || used_id != 0 {
            offline(dev);
            return 0;
        }
        dev.used_idx = dev.used_idx.wrapping_add(1);

        let take = (used_len as usize).min(req_len);
        let src = core::slice::from_raw_parts(dev.buf_virt as *const u8, take);
        out[..take].copy_from_slice(src);
        let scratch = core::slice::from_raw_parts_mut(dev.buf_virt as *mut u8, req_len);
        zeroize(scratch);
        take
    }
}

fn zeroize(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        unsafe { core::ptr::write_volatile(b, 0) };
    }
    compiler_fence(Ordering::SeqCst);
}

fn avail_offset(base: usize, queue_size: usize) -> usize {
    base + queue_size * VRING_DESC_SIZE
}

fn used_offset(base: usize, queue_size: usize) -> usize {
    (base + queue_size * VRING_DESC_SIZE + 4 + queue_size * 2 + 2 + 4095) & !4095
}

fn vring_bytes(queue_size: usize) -> usize {
    used_offset(0, queue_size) + 4 + queue_size * 8 + 2
}

fn alloc_contiguous_queue(bytes: usize) -> Option<(usize, usize)> {
    let pmm = get_pmm()?;
    let pages = (bytes + 4095) / 4096;
    if pages == 0 || pages > MAX_QUEUE_PAGES {
        return None;
    }
    let mut phys = pmm.alloc_contiguous(pages)?.as_usize();
    if phys == 0 {
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
        if inb(io_base + REG_DEVICE_STATUS) == 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn offline(dev: &mut VirtioRngDevice) {
    dev.failed = true;
    let _ = reset_device(dev.io_base);
    crate::serial_write("[VIRTIO-RNG] device offline; DMA quarantined\r\n");
}

fn fail_device(io_base: u16) {
    let status = inb(io_base + REG_DEVICE_STATUS);
    outb(io_base + REG_DEVICE_STATUS, status | STATUS_FAILED);
}

unsafe fn write_desc(base: usize, index: usize, addr: u64, len: u32, flags: u16, next: u16) {
    let ptr = base + index * VRING_DESC_SIZE;
    core::ptr::write_volatile(ptr as *mut u64, addr);
    core::ptr::write_volatile((ptr + 8) as *mut u32, len);
    core::ptr::write_volatile((ptr + 12) as *mut u16, flags);
    core::ptr::write_volatile((ptr + 14) as *mut u16, next);
}

unsafe fn read_u16(addr: usize) -> u16 {
    core::ptr::read_volatile(addr as *const u16)
}

unsafe fn write_u16(addr: usize, value: u16) {
    core::ptr::write_volatile(addr as *mut u16, value);
}

fn inw(port: u16) -> u16 {
    unsafe { hal::hal_inw(port) }
}

fn inb(port: u16) -> u8 {
    unsafe { hal::hal_inb(port) }
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
