//! HPET main counter only; PIT remains the routed scheduling timer.
use crate::memory::vmm;
use crate::sync::OnceCell;
use alloc::vec::Vec;
struct Hpet {
    mapped: usize,
    period_fs: u64,
    origin: u64,
    base_ns: u64,
}
static CLOCK: OnceCell<Hpet> = OnceCell::new();
const COUNTER: usize = 0xf0;
unsafe fn read(mapped: usize, offset: usize) -> u64 {
    core::ptr::read_volatile((mapped + offset) as *const u64)
}
unsafe fn write(mapped: usize, offset: usize, value: u64) {
    core::ptr::write_volatile((mapped + offset) as *mut u64, value);
}
fn firmware_bytes(address: u64, length: usize) -> Option<Vec<u8>> {
    if address == 0 || length == 0 || address.checked_add(length as u64)? > (1u64 << 52) {
        return None;
    }
    let mapped = vmm::map_firmware_region(address as usize, length)?;
    let mut result = Vec::with_capacity(length);
    for i in 0..length {
        result.push(unsafe { core::ptr::read_volatile((mapped + i) as *const u8) });
    }
    assert!(
        vmm::unmap_mmio_region(mapped, length),
        "firmware mapping retirement"
    );
    Some(result)
}
pub(super) fn init(base_ns: u64) -> bool {
    if CLOCK.get().is_some() {
        return true;
    }
    extern "C" {
        fn hal_rsdp_address() -> u64;
    }
    let raw = unsafe { hal_rsdp_address() };
    // Limine revisions may expose RSDP as a physical address or an HHDM pointer.
    let physical = if raw >= 0xffff800000000000 {
        vmm::virt_to_phys(raw as usize) as u64
    } else {
        raw
    };
    let Some(address) = super::acpi::hpet_address(physical, firmware_bytes) else {
        return false;
    };
    let Some(mapped) = vmm::map_mmio_region(address as usize, 4096) else {
        return false;
    };
    let capabilities = unsafe { read(mapped, 0) };
    let period_fs = capabilities >> 32;
    // HPET specifies <=100 ns periods. A 32-bit-only device would require a
    // periodic wrap extension; reject it rather than misreport elapsed time.
    if capabilities & (1 << 13) == 0 || period_fs == 0 || period_fs > 100_000_000 {
        assert!(vmm::unmap_mmio_region(mapped, 4096));
        return false;
    }
    unsafe {
        let timers = ((capabilities >> 8) & 31) + 1;
        for timer in 0..timers {
            let offset = 0x100 + timer as usize * 0x20;
            write(mapped, offset, read(mapped, offset) & !(1 << 2));
        }
        write(mapped, 0x10, (read(mapped, 0x10) & !2) | 1); // disable legacy replacement; keep PIT IRQ0
    }
    let origin = unsafe { read(mapped, COUNTER) };
    let clock = Hpet {
        mapped,
        period_fs,
        origin,
        base_ns,
    };
    // A stuck counter must not replace the functioning PIT fallback.
    let mut advanced = false;
    for _ in 0..100_000 {
        if unsafe { read(mapped, COUNTER) } != origin {
            advanced = true;
            break;
        }
        core::hint::spin_loop();
    }
    if !advanced {
        assert!(vmm::unmap_mmio_region(mapped, 4096));
        return false;
    }
    assert!(CLOCK.set(clock).is_ok(), "clock initialized twice");
    true
}
pub(super) fn now() -> Option<u64> {
    let clock = CLOCK.get()?;
    let delta = unsafe { read(clock.mapped, COUNTER) }.wrapping_sub(clock.origin);
    let ns = counter_to_ns(delta, clock.period_fs);
    Some(clock.base_ns.saturating_add(ns))
}
pub(super) fn resolution_ns() -> Option<u64> {
    Some(CLOCK.get()?.period_fs.div_ceil(1_000_000))
}

pub(super) fn counter_to_ns(delta: u64, period_fs: u64) -> u64 {
    (delta as u128 * period_fs as u128 / 1_000_000).min(u64::MAX as u128) as u64
}
