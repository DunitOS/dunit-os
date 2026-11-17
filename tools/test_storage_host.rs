//! Run the real drivers with a bounded frame pool and simulated port I/O.
#[path = "../kernel/src/sync.rs"]
mod sync;
extern crate alloc;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, OnceLock};

static TEST_LOCK: Mutex<()> = Mutex::new(());
struct AhciPort {
    base: usize,
    registers: [u32; 32],
    delay: Option<usize>,
    remaining: Option<usize>,
    task_error: bool,
    issued: usize,
}
struct VirtioCompletion {
    used: usize,
    data: usize,
    status: usize,
}
#[derive(Default)]
struct Hardware {
    next: usize,
    allocated: BTreeSet<usize>,
    freed: Vec<usize>,
    status: u8,
    stuck_reset: bool,
    notifications: usize,
    publications: usize,
    selected: u32,
    config: BTreeMap<u32, u32>,
    masks: BTreeMap<u32, u32>,
    ahci: Option<AhciPort>,
    virtio_features: u32,
    virtio_guest_features: u32,
    virtio_capacity: u64,
    virtio_queue_size: u16,
    virtio_completion: Option<VirtioCompletion>,
}
static HARDWARE: Mutex<Hardware> = Mutex::new(Hardware {
    next: 4096,
    allocated: BTreeSet::new(),
    freed: Vec::new(),
    status: 0,
    stuck_reset: false,
    notifications: 0,
    publications: 0,
    selected: 0,
    config: BTreeMap::new(),
    masks: BTreeMap::new(),
    ahci: None,
    virtio_features: 0,
    virtio_guest_features: 0,
    virtio_capacity: 1024,
    virtio_queue_size: 8,
    virtio_completion: None,
});
fn reset() {
    *HARDWARE.lock().unwrap() = Hardware {
        next: 4096,
        virtio_capacity: 1024,
        virtio_queue_size: 8,
        ..Hardware::default()
    };
}
fn pool() -> usize {
    static POOL: OnceLock<usize> = OnceLock::new();
    *POOL.get_or_init(|| unsafe {
        let ptr =
            std::alloc::alloc_zeroed(std::alloc::Layout::from_size_align(1 << 20, 4096).unwrap());
        assert!(!ptr.is_null());
        ptr as usize
    })
}
mod memory {
    pub mod pmm {
        #[derive(Clone, Copy)]
        pub struct PhysicalAddress(pub usize);
        impl PhysicalAddress {
            pub fn as_usize(self) -> usize {
                self.0
            }
            pub fn from_usize(value: usize) -> Self {
                Self(value)
            }
        }
        pub struct Pmm;
        pub fn get_pmm() -> Option<&'static Pmm> {
            Some(&Pmm)
        }
        impl Pmm {
            pub fn alloc_frame(&self) -> Option<PhysicalAddress> {
                self.alloc_contiguous(1)
            }
            pub fn alloc_contiguous(&self, pages: usize) -> Option<PhysicalAddress> {
                let mut h = crate::HARDWARE.lock().unwrap();
                let first = h.next;
                if first + pages * 4096 > 1 << 20 {
                    return None;
                }
                for page in 0..pages {
                    assert!(h.allocated.insert(first + page * 4096));
                }
                h.next += pages * 4096;
                Some(PhysicalAddress(first))
            }
            pub fn free_frame(&self, page: PhysicalAddress) {
                let mut h = crate::HARDWARE.lock().unwrap();
                assert!(h.allocated.remove(&page.0), "double free");
                h.freed.push(page.0);
            }
        }
    }
    pub mod vmm {
        pub fn phys_to_virt(phys: usize) -> usize {
            crate::pool() + phys
        }
        pub fn map_mmio_region(_: usize, _: usize) -> Option<usize> {
            None
        }
        pub fn unmap_mmio_region(_: usize, _: usize) -> bool {
            true
        }
    }
}
mod serial {
    pub fn write_dec(_: u64) {}
    pub fn write_hex(_: u64, _: usize) {}
    pub fn write_hex8(_: u8) {}
    pub fn write_hex16(_: u16) {}
}
fn serial_write(_: &str) {}
mod hal {
    pub unsafe fn mmio_read32(register: *const u32) -> u32 {
        let mut h = crate::HARDWARE.lock().unwrap();
        if let Some(port) = h.ahci.as_mut() {
            if let Some(offset) = (register as usize)
                .checked_sub(port.base)
                .filter(|off| *off < 128)
            {
                if offset == 0x38 && port.registers[0x38 / 4] & 1 != 0 {
                    match port.remaining {
                        Some(0) => port.registers[0x38 / 4] &= !1,
                        Some(n) => port.remaining = Some(n - 1),
                        None => {}
                    }
                }
                if offset == 0x10 && port.task_error && port.issued > 0 {
                    port.registers[0x10 / 4] |= 1 << 30;
                }
                return port.registers[offset / 4];
            }
        }
        core::ptr::read_volatile(register)
    }
    pub unsafe fn mmio_write32(register: *mut u32, value: u32) {
        let mut h = crate::HARDWARE.lock().unwrap();
        if let Some(port) = h.ahci.as_mut() {
            if let Some(offset) = (register as usize)
                .checked_sub(port.base)
                .filter(|off| *off < 128)
            {
                if matches!(offset, 0x10 | 0x30) {
                    port.registers[offset / 4] &= !value; // W1C status/error registers
                } else {
                    port.registers[offset / 4] = value;
                    if offset == 0x38 && value & 1 != 0 {
                        port.remaining = port.delay;
                        port.issued += 1;
                    }
                }
                return;
            }
        }
        core::ptr::write_volatile(register, value);
    }
    pub unsafe fn hal_inb(_: u16) -> u8 {
        crate::HARDWARE.lock().unwrap().status
    }
    pub unsafe fn hal_inw(_: u16) -> u16 {
        crate::HARDWARE.lock().unwrap().virtio_queue_size
    }
    pub unsafe fn hal_inl(port: u16) -> u32 {
        let h = crate::HARDWARE.lock().unwrap();
        if port == 0xcfc {
            let value = h.config.get(&h.selected).copied().unwrap_or(u32::MAX);
            if value == u32::MAX {
                h.masks.get(&h.selected).copied().unwrap_or(value)
            } else {
                value
            }
        } else {
            match port {
                0 => h.virtio_features,
                20 => h.virtio_capacity as u32,
                24 => (h.virtio_capacity >> 32) as u32,
                _ => 0,
            }
        }
    }
    pub unsafe fn hal_outb(_: u16, value: u8) {
        let mut h = crate::HARDWARE.lock().unwrap();
        if value != 0 || !h.stuck_reset {
            h.status = value;
        }
    }
    pub unsafe fn hal_outw(port: u16, value: u16) {
        let mut h = crate::HARDWARE.lock().unwrap();
        if port == 0xcfc {
            let key = h.selected;
            let current = h.config.get(&key).copied().unwrap_or(0);
            h.config
                .insert(key, (current & 0xffff0000) | u32::from(value));
        } else if port == 16 {
            h.notifications += 1;
            if let Some(completion) = &h.virtio_completion {
                core::ptr::write_bytes(completion.data as *mut u8, 0x66, 512);
                core::ptr::write_volatile(completion.status as *mut u8, 0);
                let idx = core::ptr::read_volatile((completion.used + 2) as *const u16);
                let slot = completion.used + 4 + usize::from(idx % 8) * 8;
                core::ptr::write_volatile(slot as *mut u32, 0);
                core::ptr::write_volatile((slot + 4) as *mut u32, 513);
                core::ptr::write_volatile((completion.used + 2) as *mut u16, idx.wrapping_add(1));
            }
        }
    }
    pub unsafe fn hal_outl(port: u16, value: u32) {
        let mut h = crate::HARDWARE.lock().unwrap();
        if port == 0xcf8 {
            h.selected = value;
        } else if port == 0xcfc {
            let key = h.selected;
            if key & 0xfc == 4 {
                let current = h.config.get(&key).copied().unwrap_or(0);
                h.config.insert(
                    key,
                    (current & !(value & 0xffff0000)) & 0xffff0000 | (value & 0xffff),
                );
            } else {
                h.config.insert(key, value);
            }
        } else if port == 4 {
            h.virtio_guest_features = value;
        }
        drop(h);
        std::thread::yield_now();
    }
}
mod drivers {
    pub mod block {
        include!("../kernel/src/drivers/block.rs");
    }
    pub mod registry {
        include!("../kernel/src/drivers/registry.rs");
    }
    pub mod pci {
        include!("../kernel/src/drivers/pci.rs");
    }
}
mod fs {
    pub mod vfs {
        pub fn register_device_node(_: &str) {
            let mut blocks = [None; 8];
            crate::drivers::block::snapshot(&mut blocks);
            let mut devices = [None; 32];
            crate::drivers::registry::snapshot(&mut devices);
            crate::HARDWARE.lock().unwrap().publications += 1;
        }
    }
}
fn pci_device(device: u8) -> drivers::pci::PciDevice {
    drivers::pci::PciDevice {
        bus: 0,
        device,
        function: 0,
        vendor_id: 0,
        device_id: 0,
        class_code: 0,
        subclass: 0,
        prog_if: 0,
        header_type: 0,
        interrupt_line: 0,
        interrupt_pin: 0,
        capabilities: drivers::pci::PciCapabilities {
            first_pointer: 0,
            count: 0,
            has_msi: false,
            has_msix: false,
        },
    }
}
mod ahci {
    include!("../kernel/src/drivers/ahci.rs");
    fn disk(port: usize) -> AhciDisk {
        AhciDisk {
            port,
            failed: false,
            capacity_sectors: 1024,
            command_list: alloc_dma_page(true).unwrap(),
            _received_fis: alloc_dma_page(true).unwrap(),
            command_table: alloc_dma_page(true).unwrap(),
            data: alloc_dma_pages(true).unwrap(),
        }
    }
    fn modeled_port(delay: Option<usize>, task_error: bool) -> usize {
        let base = 0x10000;
        let mut registers = [0; 32];
        registers[PX_CMD / 4] = PXCMD_ST | PXCMD_FRE;
        crate::HARDWARE.lock().unwrap().ahci = Some(crate::AhciPort {
            base,
            registers,
            delay,
            remaining: None,
            task_error,
            issued: 0,
        });
        base
    }
    fn backing(disk: &AhciDisk) -> Vec<u8> {
        let mut bytes = Vec::new();
        for page in [disk.command_list, disk._received_fis, disk.command_table]
            .iter()
            .chain(&disk.data)
        {
            bytes.extend_from_slice(unsafe {
                core::slice::from_raw_parts(page.virt as *const u8, PAGE_SIZE)
            });
        }
        bytes
    }
    #[test]
    fn delayed_command_completion_and_flush_finish_before_pages_retire() {
        let _test = crate::TEST_LOCK.lock().unwrap();
        crate::reset();
        let port = modeled_port(Some(5), false);
        DISKS.lock()[0] = Some(disk(port));
        assert_eq!(transfer_write(0, 7, &[0x55; 512]), Ok(512));
        assert_eq!(
            crate::HARDWARE
                .lock()
                .unwrap()
                .ahci
                .as_ref()
                .unwrap()
                .issued,
            2
        );
        let disk = DISKS.lock()[0].take().unwrap();
        assert!(!disk.failed);
        assert_eq!(
            unsafe { core::slice::from_raw_parts(disk.data[0].virt as *const u8, 512) },
            [0x55; 512]
        );
        assert!(crate::HARDWARE.lock().unwrap().freed.is_empty());
        retire_disk(disk);
        assert!(crate::HARDWARE.lock().unwrap().allocated.is_empty());
    }
    #[test]
    fn late_ci_completion_cannot_reenable_or_rewrite_a_timed_out_disk() {
        let _test = crate::TEST_LOCK.lock().unwrap();
        crate::reset();
        let port = modeled_port(None, false);
        DISKS.lock()[0] = Some(disk(port));
        assert_eq!(transfer_write(0, 7, &[0x55; 512]), Err(BlockError::Io));
        assert_eq!(
            AHCI_LAST_ERROR.load(Ordering::Acquire),
            AhciError::CommandTimeout.code()
        );
        let before = backing(DISKS.lock()[0].as_ref().unwrap());
        crate::HARDWARE
            .lock()
            .unwrap()
            .ahci
            .as_mut()
            .unwrap()
            .remaining = Some(0);
        assert_eq!(read32(port, PX_CI), 0); // the device completes after the caller timed out
        assert_eq!(transfer_write(0, 8, &[0xaa; 512]), Err(BlockError::Io));
        let mut read = [0xcc; 512];
        assert_eq!(transfer_read(0, 7, &mut read), Err(BlockError::Io));
        assert_eq!(read, [0xcc; 512]);
        assert_eq!(backing(DISKS.lock()[0].as_ref().unwrap()), before);
        let h = crate::HARDWARE.lock().unwrap();
        assert_eq!(h.ahci.as_ref().unwrap().issued, 1);
        assert!(h.freed.is_empty());
        drop(h);
        DISKS.lock()[0] = None;
    }
    #[test]
    fn task_file_error_with_active_ci_and_failed_stop_retains_backing() {
        let _test = crate::TEST_LOCK.lock().unwrap();
        crate::reset();
        let port = modeled_port(None, true);
        crate::HARDWARE
            .lock()
            .unwrap()
            .ahci
            .as_mut()
            .unwrap()
            .registers[PX_CMD / 4] |= PXCMD_CR | PXCMD_FR;
        let mut disk = disk(port);
        assert_eq!(
            issue_data(&mut disk, ATA_CMD_READ_DMA_EXT, 0, false, 1),
            Err(AhciError::TaskFile)
        );
        assert!(disk.failed);
        assert_eq!(read32(port, PX_CI), 1);
        let before = backing(&disk);
        assert_eq!(
            issue_nodata(&mut disk, ATA_CMD_FLUSH_CACHE_EXT),
            Err(AhciError::CommandTimeout)
        );
        assert_eq!(backing(&disk), before);
        retire_disk(disk);
        let h = crate::HARDWARE.lock().unwrap();
        assert_eq!(h.ahci.as_ref().unwrap().issued, 1);
        assert_eq!(h.allocated.len(), DATA_PAGES + 3);
        assert!(h.freed.is_empty());
    }
    #[test]
    fn busy_timeout_takes_disk_offline_before_a_retry_can_write_dma() {
        let _test = crate::TEST_LOCK.lock().unwrap();
        crate::reset();
        let mut registers = [0u32; 32];
        let port = registers.as_mut_ptr() as usize;
        write32(port, PX_TFD, TFD_BSY);
        let mut disk = disk(port);
        assert_eq!(
            issue_data(&mut disk, ATA_CMD_WRITE_DMA_EXT, 0, true, 1),
            Err(AhciError::DeviceBusy)
        );
        assert!(disk.failed);
        let table =
            unsafe { core::slice::from_raw_parts(disk.command_table.virt as *const u8, PAGE_SIZE) }
                .to_vec();
        write32(port, PX_TFD, 0); // delayed completion cannot re-enable the disk
        DISKS.lock()[0] = Some(disk);
        assert_eq!(transfer_write(0, 0, &[0x55; 512]), Err(BlockError::Io));
        let disks = DISKS.lock();
        let disk = disks[0].as_ref().unwrap();
        assert_eq!(table, unsafe {
            core::slice::from_raw_parts(disk.command_table.virt as *const u8, PAGE_SIZE)
        });
        assert!(crate::HARDWARE.lock().unwrap().freed.is_empty());
        drop(disks);
        DISKS.lock()[0] = None;
    }
    #[test]
    fn failed_stop_quarantines_all_dma_pages() {
        let _test = crate::TEST_LOCK.lock().unwrap();
        crate::reset();
        for busy in [PXCMD_CR, PXCMD_FR, PXCMD_CR | PXCMD_FR] {
            let mut registers = [0u32; 32];
            registers[PX_CMD / 4] = PXCMD_ST | PXCMD_FRE | busy;
            retire_disk(disk(registers.as_mut_ptr() as usize));
            assert!(crate::HARDWARE.lock().unwrap().freed.is_empty());
        }
    }
    #[test]
    fn stopped_port_releases_each_page_once() {
        let _test = crate::TEST_LOCK.lock().unwrap();
        crate::reset();
        let mut registers = [0u32; 32];
        retire_disk(disk(registers.as_mut_ptr() as usize));
        let h = crate::HARDWARE.lock().unwrap();
        assert_eq!(h.freed.len(), DATA_PAGES + 3);
        assert!(h.allocated.is_empty());
    }
}
mod virtio {
    include!("../kernel/src/drivers/virtio_blk.rs");
    fn device() -> VirtioBlkDevice {
        let (_, queue_virt) = alloc_contiguous_queue(vring_bytes(8)).unwrap();
        let (io_phys, io_virt) = alloc_frame_pair().unwrap();
        unsafe {
            core::ptr::write_bytes(queue_virt as *mut u8, 0, vring_bytes(8));
        }
        VirtioBlkDevice {
            io_base: 0,
            capacity_sectors: 1024,
            readonly: false,
            queue_virt,
            queue_size: 8,
            io_phys,
            io_virt,
            avail_idx: 0,
            used_idx: 0,
            failed: false,
        }
    }
    #[test]
    fn readonly_feature_rejects_writes_before_dma_and_keeps_reads_working() {
        let _test = crate::TEST_LOCK.lock().unwrap();
        crate::reset();
        {
            let mut h = crate::HARDWARE.lock().unwrap();
            h.config.insert(0x80000010, 1);
            h.virtio_features = VIRTIO_BLK_F_RO | (1 << 9);
        }
        let dev = init_device(crate::pci_device(0)).unwrap();
        assert!(dev.readonly);
        let io = dev.io_virt;
        let before = unsafe { core::slice::from_raw_parts(io as *const u8, 4096) }.to_vec();
        crate::HARDWARE.lock().unwrap().virtio_completion = Some(crate::VirtioCompletion {
            used: used_offset(dev.queue_virt, dev.queue_size),
            data: io + 64,
            status: io + 64 + SECTOR_SIZE,
        });
        *VD0.lock() = Some(dev);
        assert_eq!(vd0_write(0, &[0xaa; 1024]), Err(BlockError::Io));
        assert_eq!(before, unsafe {
            core::slice::from_raw_parts(io as *const u8, 4096)
        });
        assert_eq!(crate::HARDWARE.lock().unwrap().notifications, 0);
        let mut data = [0; 1024];
        assert_eq!(vd0_read(0, &mut data), Ok(1024));
        assert_eq!(data, [0x66; 1024]);
        assert!(!VD0.lock().as_ref().unwrap().failed);
        let h = crate::HARDWARE.lock().unwrap();
        assert_eq!(h.virtio_guest_features, VIRTIO_BLK_F_RO);
        assert_eq!(h.notifications, 2);
        drop(h);
        *VD0.lock() = None;
    }
    #[test]
    fn queue_allocation_failure_and_invalid_sizes_publish_failed_status() {
        let _test = crate::TEST_LOCK.lock().unwrap();
        for size in [2, 3, 65535, 8] {
            crate::reset();
            {
                let mut h = crate::HARDWARE.lock().unwrap();
                h.config.insert(0x80000010, 1);
                h.virtio_queue_size = size;
                if size == 8 {
                    h.next = 1 << 20;
                }
            }
            assert!(init_device(crate::pci_device(0)).is_none());
            let h = crate::HARDWARE.lock().unwrap();
            assert_eq!(h.status, STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FAILED);
            assert!(h.allocated.is_empty());
            assert_eq!(h.notifications, 0);
        }
    }
    #[test]
    fn completion_timeout_blocks_retry_even_when_reset_stays_stuck() {
        let _test = crate::TEST_LOCK.lock().unwrap();
        crate::reset();
        let dev = device();
        let queue = dev.queue_virt;
        let io = dev.io_virt;
        {
            let mut h = crate::HARDWARE.lock().unwrap();
            h.status = STATUS_DRIVER_OK;
            h.stuck_reset = true;
        }
        *VD0.lock() = Some(dev);
        assert_eq!(transfer(0, &mut [0x55; 512], true), Err(BlockError::Io));
        assert!(VD0.lock().as_ref().unwrap().failed);
        let data = unsafe { core::slice::from_raw_parts(io as *const u8, 4096) }.to_vec();
        // A device can publish a late used entry after the caller has timed out.
        unsafe {
            write_u16(used_offset(queue, 8) + 2, 1);
        }
        assert_eq!(transfer(1, &mut [0xaa; 512], true), Err(BlockError::Io));
        assert_eq!(data, unsafe {
            core::slice::from_raw_parts(io as *const u8, 4096)
        });
        let h = crate::HARDWARE.lock().unwrap();
        assert_eq!(h.notifications, 1);
        assert!(h.freed.is_empty());
        drop(h);
        *VD0.lock() = None;
    }
    #[test]
    fn reset_timeout_is_an_error_instead_of_reinitializing_a_live_device() {
        let _test = crate::TEST_LOCK.lock().unwrap();
        crate::reset();
        {
            let mut h = crate::HARDWARE.lock().unwrap();
            h.status = STATUS_DRIVER_OK;
            h.stuck_reset = true;
        }
        let pci = crate::pci_device(0);
        crate::HARDWARE.lock().unwrap().config.insert(0x80000010, 1);
        assert!(init_device(pci).is_none());
        assert_eq!(crate::HARDWARE.lock().unwrap().status, STATUS_DRIVER_OK);
        assert!(crate::HARDWARE.lock().unwrap().allocated.is_empty());
    }
    #[test]
    fn queue_at_zero_is_returned_without_leaking_or_using_the_disabled_pfn() {
        let _test = crate::TEST_LOCK.lock().unwrap();
        crate::reset();
        crate::HARDWARE.lock().unwrap().next = 0;
        let (phys, _) = alloc_contiguous_queue(8192).unwrap();
        assert_eq!(phys, 8192);
        let h = crate::HARDWARE.lock().unwrap();
        assert_eq!(h.freed, [0, 4096]);
        assert_eq!(h.allocated.len(), 2);
    }
    #[test]
    fn invalid_used_id_cannot_release_an_unrelated_chain() {
        let _test = crate::TEST_LOCK.lock().unwrap();
        crate::reset();
        let dev = device();
        unsafe {
            write_u16(used_offset(dev.queue_virt, 8) + 2, 1);
            write_u32(used_offset(dev.queue_virt, 8) + 4, 7);
        }
        *VD0.lock() = Some(dev);
        assert_eq!(transfer(0, &mut [0; 512], false), Err(BlockError::Io));
        assert!(VD0.lock().as_ref().unwrap().failed);
        *VD0.lock() = None;
    }
    #[test]
    fn invalid_full_range_never_submits_a_partial_write_or_read() {
        let _test = crate::TEST_LOCK.lock().unwrap();
        crate::reset();
        *VD0.lock() = Some(device());
        for lba in [1023, 1024, u64::MAX] {
            assert_eq!(vd0_write(lba, &[0x55; 1024]), Err(BlockError::OutOfRange));
            let mut data = [0xaa; 1024];
            assert_eq!(vd0_read(lba, &mut data), Err(BlockError::OutOfRange));
            assert_eq!(data, [0xaa; 1024]);
        }
        assert_eq!(crate::HARDWARE.lock().unwrap().notifications, 0);
        assert!(!VD0.lock().as_ref().unwrap().failed);
        *VD0.lock() = None;
    }
}
#[test]
fn readonly_block_registration_never_dispatches_a_write_callback() {
    let _test = TEST_LOCK.lock().unwrap();
    reset();
    use drivers::block::{BlockDeviceInfo, BlockError};
    fn read(_: u64, data: &mut [u8]) -> Result<usize, BlockError> {
        data.fill(0x77);
        Ok(data.len())
    }
    fn write(_: u64, _: &[u8]) -> Result<usize, BlockError> {
        panic!("readonly write dispatched")
    }
    drivers::block::register_device(
        BlockDeviceInfo {
            name: "readonly",
            driver: "fixture",
            block_size: 512,
            blocks: 10,
            readonly: true,
        },
        read,
        write,
    );
    assert_eq!(
        drivers::block::write_block("readonly", 0, &[0x55; 512]),
        Err(BlockError::Io)
    );
    let mut data = [0; 512];
    assert_eq!(
        drivers::block::read_block("readonly", 0, &mut data),
        Ok(512)
    );
    assert_eq!(data, [0x77; 512]);
}
#[test]
fn block_callbacks_and_publication_can_reenter_the_registry() {
    let _test = TEST_LOCK.lock().unwrap();
    reset();
    fn read(_: u64, buf: &mut [u8]) -> Result<usize, drivers::block::BlockError> {
        let mut snapshot = [None; 8];
        drivers::block::snapshot(&mut snapshot);
        buf.fill(0x55);
        Ok(buf.len())
    }
    fn write(_: u64, buf: &[u8]) -> Result<usize, drivers::block::BlockError> {
        Ok(buf.len())
    }
    let info = drivers::block::BlockDeviceInfo {
        name: "test",
        driver: "test",
        block_size: 512,
        blocks: 100,
        readonly: false,
    };
    drivers::block::register_device(info, read, write);
    drivers::block::register_device(info, read, write);
    let mut data = [0; 512];
    assert_eq!(drivers::block::read_block("test", 0, &mut data), Ok(512));
    assert!(data.iter().all(|byte| *byte == 0x55));
    assert_eq!(HARDWARE.lock().unwrap().publications, 2);
}

#[test]
fn pci_address_data_pairs_and_command_updates_survive_contending_readers() {
    let _test = TEST_LOCK.lock().unwrap();
    reset();
    for device in 0..4 {
        HARDWARE
            .lock()
            .unwrap()
            .config
            .insert(0x80000004 | device << 11, 0xa0000000);
    }
    std::thread::scope(|scope| {
        for device in 0..4u8 {
            scope.spawn(move || {
                let pci = pci_device(device);
                for iteration in 0..2000 {
                    drivers::pci::enable_mmio_bus_master(pci);
                    assert_eq!(
                        drivers::pci::read_config(0, device, 0, 4),
                        if iteration == 0 {
                            0xa0000006
                        } else {
                            0xa0000007
                        }
                    );
                    drivers::pci::enable_io_bus_master(pci);
                    assert_eq!(drivers::pci::read_config(0, device, 0, 4), 0xa0000007);
                }
            });
        }
    });
}

#[test]
fn pci_bar_probe_restores_address_and_keeps_pending_w1c_status() {
    let _test = TEST_LOCK.lock().unwrap();
    reset();
    {
        let mut h = HARDWARE.lock().unwrap();
        h.config.insert(0x80000004, 0xa0000007);
        h.config.insert(0x80000010, 0xc0000000);
        h.masks.insert(0x80000010, 0xff000000);
    }
    let info = drivers::pci::read_bar_info(pci_device(0), 0);
    assert_eq!(info.bar, drivers::pci::PciBar::Memory32(0xc0000000));
    assert_eq!(info.size, 1 << 24);
    assert_eq!(drivers::pci::read_config(0, 0, 0, 4), 0xa0000007);
    assert_eq!(drivers::pci::read_config(0, 0, 0, 0x10), 0xc0000000);
}

mod initrd {
    include!("../kernel/src/initrd.rs");
    fn archive() -> &'static [u8] {
        let mut bytes = b"DUNITRD1".to_vec();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&9u32.to_le_bytes());
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(b"/app/test");
        bytes.extend_from_slice(b"ELF");
        Box::leak(bytes.into_boxed_slice())
    }
    #[test]
    fn every_truncation_is_rejected_and_complete_data_is_zero_copy() {
        let bytes = archive();
        for length in 0..bytes.len() {
            assert!(parse_archive(&bytes[..length]).is_none());
        }
        let parsed = parse_archive(bytes).unwrap();
        assert_eq!(parsed.file_count(), 1);
        assert_eq!(parsed.entries()[0].name, "/app/test");
        assert_eq!(parsed.entries()[0].data.as_ptr(), bytes[29..].as_ptr());
    }
    #[test]
    fn repeat_initialization_and_parallel_readers_keep_one_immutable_archive() {
        let _test = crate::TEST_LOCK.lock().unwrap();
        let bytes = archive();
        set_module(bytes.as_ptr(), bytes.len());
        assert_eq!(init(), 1);
        let pointer = get_initrd().unwrap() as *const Initrd as usize;
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(move || {
                    for _ in 0..1000 {
                        assert_eq!(init(), 1);
                        assert_eq!(file_count(), 1);
                        assert_eq!(get_initrd().unwrap() as *const Initrd as usize, pointer);
                    }
                });
            }
        });
    }
}
