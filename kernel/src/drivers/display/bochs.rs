//! Бэкенд Bochs VBE (DISPI / BGA).
//!
//! QEMU `-device VGA` (std VGA) реализует этот legacy-интерфейс: смена режима на
//! ходу через порты ввода-вывода 0x01CE (индекс) / 0x01CF (данные). База
//! линейного фреймбуфера берётся из PCI BAR0. Маппинг всей доступной VRAM
//! создаётся до запуска userspace, поэтому режим не ограничен boot-разрешением.
//! Без подходящего PCI BAR сохраняется безопасный бюджет маппинга Limine.

use super::{DisplayError, Framebuffer, Mode};
use crate::{drivers::pci, hal, memory::vmm};

const VBE_DISPI_IOPORT_INDEX: u16 = 0x01CE;
const VBE_DISPI_IOPORT_DATA: u16 = 0x01CF;

const VBE_DISPI_INDEX_ID: u16 = 0;
const VBE_DISPI_INDEX_XRES: u16 = 1;
const VBE_DISPI_INDEX_YRES: u16 = 2;
const VBE_DISPI_INDEX_BPP: u16 = 3;
const VBE_DISPI_INDEX_ENABLE: u16 = 4;
const VBE_DISPI_INDEX_VIRT_WIDTH: u16 = 6;
const VBE_DISPI_INDEX_X_OFFSET: u16 = 8;
const VBE_DISPI_INDEX_Y_OFFSET: u16 = 9;
const VBE_DISPI_INDEX_VIDEO_MEMORY_64K: u16 = 10;
const VBE_DISPI_GETCAPS: u16 = 2;

const VBE_DISPI_DISABLED: u16 = 0x00;
const VBE_DISPI_ENABLED: u16 = 0x01;
const VBE_DISPI_LFB_ENABLED: u16 = 0x40;

const VBE_DISPI_BPP_32: u16 = 32;

/// Common scanout modes, filtered against actual device limits and VRAM.
const CANDIDATE_MODES: &[(u32, u32)] = &[
    (1024, 768),
    (1280, 720),
    (1366, 768),
    (1600, 900),
    (1920, 1080),
    (2560, 1440),
    (2560, 1600),
    (2880, 1800),
    (3072, 1920),
];

fn write_reg(index: u16, value: u16) {
    unsafe {
        hal::hal_outw(VBE_DISPI_IOPORT_INDEX, index);
        hal::hal_outw(VBE_DISPI_IOPORT_DATA, value);
    }
}

fn read_reg(index: u16) -> u16 {
    unsafe {
        hal::hal_outw(VBE_DISPI_IOPORT_INDEX, index);
        hal::hal_inw(VBE_DISPI_IOPORT_DATA)
    }
}

struct MmioMapping {
    address: usize,
    bytes: usize,
}

impl Drop for MmioMapping {
    fn drop(&mut self) {
        assert!(
            vmm::unmap_mmio_region(self.address, self.bytes),
            "display MMIO retirement failed"
        );
    }
}

enum Backing {
    Boot(usize),
    Mmio(MmioMapping),
}

impl Backing {
    fn address(&self) -> usize {
        match self {
            Self::Boot(address) => *address,
            Self::Mmio(mapping) => mapping.address,
        }
    }
}

pub struct BochsVbe {
    backing: Backing,
    /// Maximum scanout pixels within the mapped framebuffer.
    budget_px: u32,
    max_w: u32,
    max_h: u32,
}

impl BochsVbe {
    /// Детектит BGA по регистру ID (0xB0C0..=0xB0C5).
    pub(super) fn detect(boot: Framebuffer) -> Option<Self> {
        let id = read_reg(VBE_DISPI_INDEX_ID);
        if !(0xB0C0..=0xB0C5).contains(&id) {
            return None;
        }
        let enabled = read_reg(VBE_DISPI_INDEX_ENABLE);
        write_reg(VBE_DISPI_INDEX_ENABLE, enabled | VBE_DISPI_GETCAPS);
        let max_w = read_reg(VBE_DISPI_INDEX_XRES) as u32;
        let max_h = read_reg(VBE_DISPI_INDEX_YRES) as u32;
        write_reg(VBE_DISPI_INDEX_ENABLE, enabled);
        let mut budget_px = (boot.byte_len()? / 4).min(u32::MAX as usize) as u32;
        let mut mapping_owner = None;
        // DISPI is also available without PCI. Only claim extra memory when a
        // matching VGA resource is measured and successfully mapped.
        pci::for_each_device(|dev| {
            if dev.vendor_id != 0x1234 || dev.device_id != 0x1111 || dev.class_code != 3 {
                return;
            }
            let resource = pci::read_bar_info(dev, 0);
            let physical = match resource.bar {
                pci::PciBar::Memory32(p) => p as u64,
                pci::PciBar::Memory64(p) => p,
                _ => return,
            };
            let reported = read_reg(VBE_DISPI_INDEX_VIDEO_MEMORY_64K) as u64 * 65536;
            let bytes = resource.size.min(reported);
            if mapping_owner.is_some()
                || bytes < boot.byte_len().unwrap() as u64
                || bytes > usize::MAX as u64
            {
                return;
            }
            if let Some(mapping) =
                vmm::map_framebuffer_region(physical as usize, bytes as usize, boot.address)
            {
                mapping_owner = Some(MmioMapping {
                    address: mapping,
                    bytes: bytes as usize,
                });
                budget_px = (bytes / 4).min(u32::MAX as u64) as u32;
            }
        });
        Some(BochsVbe {
            backing: mapping_owner.map_or(Backing::Boot(boot.address), Backing::Mmio),
            budget_px,
            max_w,
            max_h,
        })
    }

    fn accepts(&self, w: u32, h: u32) -> bool {
        w != 0
            && h != 0
            && w <= self.max_w
            && h <= self.max_h
            && w as u64 * h as u64 <= self.budget_px as u64
    }

    pub fn query_modes(&self, out: &mut [Mode]) -> usize {
        let mut n = 0;
        for &(w, h) in CANDIDATE_MODES.iter() {
            if n >= out.len() {
                break;
            }
            if self.accepts(w, h) {
                out[n] = Mode {
                    width: w,
                    height: h,
                };
                n += 1;
            }
        }
        n
    }

    pub(super) fn boot_framebuffer(&self, boot: Framebuffer) -> Framebuffer {
        Framebuffer {
            address: self.backing.address(),
            ..boot
        }
    }

    pub(super) fn framebuffer(&self, width: u32, height: u32) -> Framebuffer {
        let address = self.backing.address();
        Framebuffer {
            address,
            width,
            height,
            pitch: width * 4,
        }
    }

    pub fn set_mode(&mut self, width: u32, height: u32) -> Result<(), DisplayError> {
        if !self.accepts(width, height) {
            return Err(DisplayError::InvalidMode);
        }
        let previous = [
            read_reg(VBE_DISPI_INDEX_XRES),
            read_reg(VBE_DISPI_INDEX_YRES),
            read_reg(VBE_DISPI_INDEX_BPP),
            read_reg(VBE_DISPI_INDEX_ENABLE),
            read_reg(VBE_DISPI_INDEX_VIRT_WIDTH),
            read_reg(VBE_DISPI_INDEX_X_OFFSET),
            read_reg(VBE_DISPI_INDEX_Y_OFFSET),
        ];
        if program_mode([
            width as u16,
            height as u16,
            VBE_DISPI_BPP_32,
            VBE_DISPI_ENABLED | VBE_DISPI_LFB_ENABLED,
            width as u16,
            0,
            0,
        ]) {
            return Ok(());
        }
        if program_mode(previous) {
            Err(DisplayError::Io)
        } else {
            Err(DisplayError::DeviceLost)
        }
    }

    pub fn present(&mut self, _rect: Option<(u32, u32, u32, u32)>) -> Result<(), DisplayError> {
        // Limine/Bochs scan the device backing directly; no flush command exists.
        Ok(())
    }
}

fn program_mode(registers: [u16; 7]) -> bool {
    write_reg(VBE_DISPI_INDEX_ENABLE, VBE_DISPI_DISABLED);
    write_reg(VBE_DISPI_INDEX_XRES, registers[0]);
    write_reg(VBE_DISPI_INDEX_YRES, registers[1]);
    write_reg(VBE_DISPI_INDEX_BPP, registers[2]);
    write_reg(VBE_DISPI_INDEX_VIRT_WIDTH, registers[4]);
    write_reg(VBE_DISPI_INDEX_X_OFFSET, registers[5]);
    write_reg(VBE_DISPI_INDEX_Y_OFFSET, registers[6]);
    write_reg(VBE_DISPI_INDEX_ENABLE, registers[3]);
    read_reg(VBE_DISPI_INDEX_XRES) == registers[0]
        && read_reg(VBE_DISPI_INDEX_YRES) == registers[1]
        && read_reg(VBE_DISPI_INDEX_BPP) == registers[2]
        && read_reg(VBE_DISPI_INDEX_ENABLE) == registers[3]
        && read_reg(VBE_DISPI_INDEX_VIRT_WIDTH) == registers[4]
        && read_reg(VBE_DISPI_INDEX_X_OFFSET) == registers[5]
        && read_reg(VBE_DISPI_INDEX_Y_OFFSET) == registers[6]
}
