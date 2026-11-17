use crate::drivers::pci::{self, PciBar, PciDevice};
use crate::drivers::registry::{self, DeviceClass};
use crate::memory::vmm;
use crate::serial::{write_dec, write_hex, write_hex16, write_hex8, write_mac};
use crate::serial_write;
use crate::sync::SpinLock;

const PCI_CLASS_NETWORK: u8 = 0x02;
const VENDOR_INTEL: u16 = 0x8086;
const VENDOR_REALTEK: u16 = 0x10ec;
const VENDOR_REDHAT: u16 = 0x1af4;
const E1000_MMIO_MAP_SIZE: usize = 0x20000;
const E1000_STATUS: usize = 0x0008;
const E1000_RAL0: usize = 0x5400;
const E1000_RAH0: usize = 0x5404;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NicKind {
    E1000,
    Rtl8139,
    VirtioNet,
    Unknown,
}

#[derive(Clone, Copy)]
pub struct NicInfo {
    pub pci: PciDevice,
    pub kind: NicKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetSnapshot {
    pub total_nics: usize,
    pub supported_nics: usize,
    pub mmio_ready_nics: usize,
    pub mac_ready_nics: usize,
}

static NET_SNAPSHOT: SpinLock<NetSnapshot> = SpinLock::new(NetSnapshot {
    total_nics: 0,
    supported_nics: 0,
    mmio_ready_nics: 0,
    mac_ready_nics: 0,
});

pub fn init() {
    let mut index = 0usize;
    let mut total = 0usize;
    let mut supported = 0usize;
    let mut mmio_ready = 0usize;
    let mut mac_ready = 0usize;

    crate::drivers::pci::scan(|dev| {
        if dev.class_code != PCI_CLASS_NETWORK {
            return;
        }

        total += 1;
        let kind = classify_nic(dev);
        if kind != NicKind::Unknown {
            supported += 1;
        }

        if index == 0 {
            registry::register("net0", DeviceClass::Network, driver_name(kind));
        }

        serial_write("[NET] pci nic ");
        write_pci_addr(dev);
        serial_write(" vendor=");
        write_hex16(dev.vendor_id);
        serial_write(" device=");
        write_hex16(dev.device_id);
        serial_write(" kind=");
        serial_write(kind_name(kind));
        serial_write("\r\n");

        match probe_nic(dev, kind) {
            Ok(probe) => {
                if probe.mmio_ready {
                    mmio_ready += 1;
                }
                if probe.mac_valid {
                    mac_ready += 1;
                }
            }
            Err(error) => {
                serial_write("[NET] probe failed kind=");
                serial_write(kind_name(kind));
                serial_write(" error=");
                serial_write(error.as_str());
                serial_write("\r\n");
            }
        }

        index += 1;
    });

    *NET_SNAPSHOT.lock() = NetSnapshot {
        total_nics: total,
        supported_nics: supported,
        mmio_ready_nics: mmio_ready,
        mac_ready_nics: mac_ready,
    };

    if total == 0 {
        serial_write("[NET] no PCI network controller detected\r\n");
    } else {
        serial_write("[NET] discovery ready nics=");
        write_dec(total as u64);
        serial_write(" supported=");
        write_dec(supported as u64);
        serial_write(" mmio-ready=");
        write_dec(mmio_ready as u64);
        serial_write(" mac-ready=");
        write_dec(mac_ready as u64);
        serial_write(" stack=not-implemented\r\n");
    }
}

pub fn snapshot() -> NetSnapshot {
    *NET_SNAPSHOT.lock()
}

fn classify_nic(dev: PciDevice) -> NicKind {
    match (dev.vendor_id, dev.device_id) {
        (VENDOR_INTEL, 0x100e | 0x100f | 0x10d3) => NicKind::E1000,
        (VENDOR_REALTEK, 0x8139) => NicKind::Rtl8139,
        (VENDOR_REDHAT, 0x1000) => NicKind::VirtioNet,
        _ => NicKind::Unknown,
    }
}

struct ProbeResult {
    mmio_ready: bool,
    mac_valid: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProbeError {
    Unsupported,
    NoMmioBar,
    MmioMap,
    InvalidRegisters,
}

impl ProbeError {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::NoMmioBar => "no-mmio-bar",
            Self::MmioMap => "mmio-map",
            Self::InvalidRegisters => "invalid-registers",
        }
    }
}

fn probe_nic(dev: PciDevice, kind: NicKind) -> Result<ProbeResult, ProbeError> {
    match kind {
        NicKind::E1000 => probe_e1000(dev),
        NicKind::Rtl8139 | NicKind::VirtioNet | NicKind::Unknown => Err(ProbeError::Unsupported),
    }
}

fn probe_e1000(dev: PciDevice) -> Result<ProbeResult, ProbeError> {
    let registers = MappedRegisters::open(dev)?;
    let mmio_phys = registers.physical;
    let mmio_virt = registers.base;

    let status = read32(mmio_virt, E1000_STATUS);
    if status == 0 || status == 0xFFFF_FFFF {
        return Err(ProbeError::InvalidRegisters);
    }

    let ral = read32(mmio_virt, E1000_RAL0);
    let rah = read32(mmio_virt, E1000_RAH0);
    let mac_valid = (rah & (1 << 31)) != 0 && (ral != 0 || (rah & 0xFFFF) != 0);

    serial_write("[NET:e1000] mmio=");
    write_hex(mmio_phys, 8);
    serial_write(" status=");
    write_hex(status as u64, 8);
    serial_write(" mac=");
    write_mac(ral, rah);
    serial_write(" packet-io=not-implemented\r\n");

    Ok(ProbeResult {
        mmio_ready: true,
        mac_valid,
    })
}

fn find_mmio_bar(dev: PciDevice) -> Result<u64, ProbeError> {
    let mut index = 0u8;
    while index < 6 {
        match pci::read_bar_decoded(dev, index) {
            PciBar::Memory32(addr) if addr != 0 => return Ok(addr as u64),
            PciBar::Memory64(addr) if addr != 0 => return Ok(addr),
            PciBar::Memory64(_) => {
                index += 2;
                continue;
            }
            PciBar::None | PciBar::Io(_) | PciBar::Memory32(_) => {}
        }
        index += 1;
    }
    Err(ProbeError::NoMmioBar)
}

fn driver_name(kind: NicKind) -> &'static str {
    match kind {
        NicKind::E1000 => "e1000-discovery",
        NicKind::Rtl8139 => "rtl8139-discovery",
        NicKind::VirtioNet => "virtio-net-discovery",
        NicKind::Unknown => "net-discovery",
    }
}

fn kind_name(kind: NicKind) -> &'static str {
    match kind {
        NicKind::E1000 => "e1000",
        NicKind::Rtl8139 => "rtl8139",
        NicKind::VirtioNet => "virtio-net",
        NicKind::Unknown => "unknown",
    }
}

fn write_pci_addr(dev: PciDevice) {
    write_hex8(dev.bus);
    serial_write(":");
    write_hex8(dev.device);
    serial_write(".");
    write_hex(dev.function as u64, 1);
}

fn read32(base: usize, offset: usize) -> u32 {
    unsafe { core::ptr::read_volatile((base + offset) as *const u32) }
}

// Packet bring-up remains private to boot diagnostics until N0 IRQ delivery and
// the userspace packet-capability contract are implemented and verified.
#[cfg(feature = "boot-smoke-tests")]
#[path = "net/dma.rs"]
mod dma;
#[cfg(feature = "boot-smoke-tests")]
#[path = "net/e1000.rs"]
mod e1000;
#[cfg(feature = "boot-smoke-tests")]
#[path = "net/rings.rs"]
mod rings;

struct MappedRegisters {
    base: usize,
    physical: u64,
    device: PciDevice,
    original_command: u16,
}
impl MappedRegisters {
    fn open(device: PciDevice) -> Result<Self, ProbeError> {
        let physical = find_mmio_bar(device)?;
        let original_command = pci::command(device);
        // Register discovery needs MEMORY_SPACE, never DMA authority. Keep
        // firmware bus mastering disabled until an owning driver publishes DMA.
        pci::set_command(device, (original_command | 2) & !4);
        match vmm::map_mmio_region(physical as usize, E1000_MMIO_MAP_SIZE) {
            Some(base) => Ok(Self {
                base,
                physical,
                device,
                original_command,
            }),
            None => {
                pci::set_command(device, original_command & !4);
                Err(ProbeError::MmioMap)
            }
        }
    }
}
#[cfg(feature = "boot-smoke-tests")]
impl e1000::Registers for MappedRegisters {
    fn read(&mut self, offset: usize) -> u32 {
        read32(self.base, offset)
    }
    fn write(&mut self, offset: usize, value: u32) {
        unsafe {
            crate::hal::mmio_write32((self.base + offset) as *mut u32, value);
        }
    }
    fn bus_master(&mut self, enabled: bool) -> bool {
        let command = pci::command(self.device);
        let requested = if enabled { command | 4 } else { command & !4 };
        pci::set_command(self.device, requested);
        (pci::command(self.device) & 4 != 0) == enabled
    }
    fn now_ns(&self) -> u64 {
        crate::clock::monotonic_ns()
    }
}
impl Drop for MappedRegisters {
    fn drop(&mut self) {
        // Never restore ambient firmware DMA without a vetted queue owner.
        pci::set_command(self.device, self.original_command & !4);
        let _ = vmm::unmap_mmio_region(self.base, E1000_MMIO_MAP_SIZE);
    }
}

/// Actual bounded MAC-local packet I/O, not a network service or an IRQ proof.
/// Run after the monotonic clock has been initialized, with no live netd owner.
#[cfg(feature = "boot-smoke-tests")]
pub fn run_packet_smoke() -> bool {
    let mut reference = None;
    pci::for_each_device(|dev| {
        if reference.is_none() && (dev.vendor_id, dev.device_id) == (VENDOR_INTEL, 0x100e) {
            reference = Some(dev);
        }
    });
    let Some(device) = reference else {
        serial_write("[NET-N0] no 82540EM reference; diagnostic skipped\r\n");
        return true;
    };
    let result = (|| {
        let registers = MappedRegisters::open(device).map_err(|_| e1000::Error::Registers)?;
        let (_, before) = crate::memory::pmm::stats_bytes();
        let mut controller = e1000::Controller::start(registers)?;
        let packets = controller.diagnostic();
        if packets.is_err() {
            let state = controller.snapshot();
            serial_write("[NET-N0] diagnostic-state sequence=");
            write_dec(state.sequence as u64);
            serial_write(" tx-completed=");
            write_dec(state.tx_completed);
            serial_write(" rx-packets=");
            write_dec(state.rx_packets);
            serial_write(" rx-dropped=");
            write_dec(state.rx_dropped);
            for (offset, value) in state.registers {
                serial_write(" reg[");
                write_hex(offset as u64, 4);
                serial_write("]=");
                write_hex(value as u64, 8);
            }
            serial_write("\r\n");
        }
        let stopped = controller.stop();
        drop(controller);
        packets?;
        stopped?;
        let (_, after) = crate::memory::pmm::stats_bytes();
        if before != after {
            return Err(e1000::Error::Stop);
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            serial_write("[NET-N0] e1000 PHY-loopback packets=");
            write_dec(e1000::TEST_PACKETS as u64);
            serial_write(" bytes=");
            write_dec((e1000::TEST_PACKETS * 128) as u64);
            serial_write(" ring-wrap, payload, reset and DMA reclaim OK\r\n");
            true
        }
        Err(reason) => {
            serial_write("[NET-N0] FAIL ");
            serial_write(match reason {
                e1000::Error::Reset => "reset",
                e1000::Error::NoMemory => "no-memory",
                e1000::Error::Registers => "registers",
                e1000::Error::BusMaster => "bus-master",
                e1000::Error::Transmit => "transmit",
                e1000::Error::Receive => "receive",
                e1000::Error::Timeout => "timeout",
                e1000::Error::Payload => "payload",
                e1000::Error::Stop => "stop",
            });
            serial_write("\r\n");
            false
        }
    }
}
