//! Бэкенд Bochs VBE (DISPI / BGA).
//!
//! QEMU `-device VGA` (std VGA) реализует этот legacy-интерфейс: смена режима на
//! ходу через порты ввода-вывода 0x01CE (индекс) / 0x01CF (данные). База
//! линейного фреймбуфера берётся из PCI BAR0. Маппинг всей доступной VRAM
//! создаётся до запуска userspace, поэтому режим не ограничен boot-разрешением.
//! Без подходящего PCI BAR сохраняется безопасный бюджет маппинга Limine.

use super::{DisplayError, Mode};
use crate::{hal, drivers::pci, memory::vmm};
use crate::syscall::{KERNEL_FB_ADDR, KERNEL_FB_HEIGHT, KERNEL_FB_PITCH, KERNEL_FB_WIDTH};
use core::sync::atomic::Ordering;

const VBE_DISPI_IOPORT_INDEX: u16 = 0x01CE;
const VBE_DISPI_IOPORT_DATA: u16 = 0x01CF;

const VBE_DISPI_INDEX_ID: u16 = 0;
const VBE_DISPI_INDEX_XRES: u16 = 1;
const VBE_DISPI_INDEX_YRES: u16 = 2;
const VBE_DISPI_INDEX_BPP: u16 = 3;
const VBE_DISPI_INDEX_ENABLE: u16 = 4;
const VBE_DISPI_INDEX_VIDEO_MEMORY_64K: u16 = 10;
const VBE_DISPI_GETCAPS: u16 = 2;

const VBE_DISPI_DISABLED: u16 = 0x00;
const VBE_DISPI_ENABLED: u16 = 0x01;
const VBE_DISPI_LFB_ENABLED: u16 = 0x40;

const VBE_DISPI_BPP_32: u16 = 32;

/// Common scanout modes, filtered against actual device limits and VRAM.
const CANDIDATE_MODES: &[(u32, u32)] = &[
    (1024, 768), (1280, 720), (1366, 768), (1600, 900), (1920, 1080),
    (2560, 1440), (2560, 1600), (2880, 1800), (3072, 1920),
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

pub struct BochsVbe {
    /// Maximum scanout pixels within the mapped framebuffer.
    budget_px: u32,
    max_w: u32,
    max_h: u32,
}

impl BochsVbe {
    /// Детектит BGA по регистру ID (0xB0C0..=0xB0C5).
    pub fn detect(boot_w: u32, boot_h: u32) -> Option<Self> {
        let id = read_reg(VBE_DISPI_INDEX_ID);
        if !(0xB0C0..=0xB0C5).contains(&id) {
            return None;
        }
        let enabled = read_reg(VBE_DISPI_INDEX_ENABLE);
        write_reg(VBE_DISPI_INDEX_ENABLE, enabled | VBE_DISPI_GETCAPS);
        let max_w = read_reg(VBE_DISPI_INDEX_XRES) as u32;
        let max_h = read_reg(VBE_DISPI_INDEX_YRES) as u32;
        write_reg(VBE_DISPI_INDEX_ENABLE, enabled);
        let mut budget_px = boot_w.saturating_mul(boot_h);
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
            if bytes < boot_w as u64 * boot_h as u64 * 4 || bytes > usize::MAX as u64 {
                return;
            }
            if let Some(mapping) = vmm::map_framebuffer_region(physical as usize, bytes as usize,
                KERNEL_FB_ADDR.load(Ordering::Relaxed) as usize) {
                KERNEL_FB_ADDR.store(mapping as u64, Ordering::Relaxed);
                budget_px = (bytes / 4).min(u32::MAX as u64) as u32;
            }
        });
        Some(BochsVbe { budget_px, max_w, max_h })
    }

    fn accepts(&self, w: u32, h: u32) -> bool {
        w != 0 && h != 0 && w <= self.max_w && h <= self.max_h
            && w as u64 * h as u64 <= self.budget_px as u64
    }

    pub fn query_modes(&self, out: &mut [Mode]) -> usize {
        let mut n = 0;
        for &(w, h) in CANDIDATE_MODES.iter() {
            if n >= out.len() {
                break;
            }
            if self.accepts(w, h) {
                out[n] = Mode { width: w, height: h };
                n += 1;
            }
        }
        n
    }

    pub fn set_mode(&mut self, w: u32, h: u32) -> Result<(), DisplayError> {
        if !self.accepts(w, h) {
            return Err(DisplayError::InvalidMode);
        }
        write_reg(VBE_DISPI_INDEX_ENABLE, VBE_DISPI_DISABLED);
        write_reg(VBE_DISPI_INDEX_XRES, w as u16);
        write_reg(VBE_DISPI_INDEX_YRES, h as u16);
        write_reg(VBE_DISPI_INDEX_BPP, VBE_DISPI_BPP_32);
        write_reg(
            VBE_DISPI_INDEX_ENABLE,
            VBE_DISPI_ENABLED | VBE_DISPI_LFB_ENABLED,
        );
        // Проверяем, что устройство приняло геометрию.
        if read_reg(VBE_DISPI_INDEX_XRES) != w as u16 || read_reg(VBE_DISPI_INDEX_YRES) != h as u16 {
            return Err(DisplayError::Io);
        }
        // База LFB неизменна; двигаются только размеры scanout. Обновляем глобалы,
        // чтобы все пути отрисовки (sys_fb_present) использовали новую геометрию.
        // KERNEL_FB_ADDR remains the kernel-owned framebuffer mapping.
        KERNEL_FB_WIDTH.store(w, Ordering::Relaxed);
        KERNEL_FB_HEIGHT.store(h, Ordering::Relaxed);
        KERNEL_FB_PITCH.store(w.saturating_mul(4), Ordering::Relaxed);
        crate::serial_write("[DISPLAY] bochs set_mode ");
        crate::serial::write_dec(w as u64);
        crate::serial_write("x");
        crate::serial::write_dec(h as u64);
        crate::serial_write("\r\n");
        Ok(())
    }

    pub fn present(&mut self, _rect: Option<(u32, u32, u32, u32)>) -> Result<(), DisplayError> {
        // std VGA сканирует KERNEL_FB_ADDR напрямую — выталкивать нечего.
        Ok(())
    }
}
