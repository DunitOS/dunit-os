//! Monotonic clock and deadline interface. PIT is the first clock source;
//! HPET is preferred after validated ACPI discovery; PIT remains the fallback
//! and scheduling event source. No cross-CPU TSC synchronization is assumed.
#[path = "clock/acpi.rs"]
mod acpi;
#[path = "clock/hpet.rs"]
mod hpet;
use core::sync::atomic::{AtomicI64, AtomicU64, Ordering};

pub const TICKS_PER_SECOND: u64 = 100;
const PIT_DIVISOR: u16 = 11_931;
static BOOT_EPOCH_SECONDS: AtomicI64 = AtomicI64::new(-1);
static MONOTONIC_TICKS: AtomicU64 = AtomicU64::new(0);

pub fn init_pit() {
    BOOT_EPOCH_SECONDS.store(
        unsafe { crate::hal::hal_boot_epoch_seconds() },
        Ordering::Relaxed,
    );
    let base_ns = MONOTONIC_TICKS
        .load(Ordering::Relaxed)
        .saturating_mul(1_000_000_000 / TICKS_PER_SECOND);
    let high_resolution = hpet::init(base_ns);
    let mut line = crate::FmtBuf::new();
    line.push_str(if high_resolution {
        "[CLOCK] source=hpet resolution_ns="
    } else {
        "[CLOCK] source=pit resolution_ns="
    });
    line.push_u64(hpet::resolution_ns().unwrap_or(1_000_000_000 / TICKS_PER_SECOND));
    line.push_str(" event=pit-100hz\r\n");
    crate::memory::serial_write(line.as_str());
    unsafe {
        crate::hal::hal_outb(0x43, 0x36);
        crate::hal::hal_outb(0x40, PIT_DIVISOR as u8);
        crate::hal::hal_outb(0x40, (PIT_DIVISOR >> 8) as u8);
    }
}

/// Called exactly once for each IRQ0, regardless of whether a task is active.
#[inline]
pub fn on_tick() {
    MONOTONIC_TICKS.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn monotonic_ticks() -> u64 {
    monotonic_ns() / (1_000_000_000 / TICKS_PER_SECOND)
}

pub fn monotonic_ns() -> u64 {
    hpet::now().unwrap_or_else(|| {
        MONOTONIC_TICKS
            .load(Ordering::Relaxed)
            .saturating_mul(1_000_000_000 / TICKS_PER_SECOND)
    })
}

#[derive(Clone, Copy)]
pub struct Deadline(u64);

impl Deadline {
    pub fn after_ms(ms: u64) -> Self {
        let duration = if ms == 0 {
            1_000_000_000 / TICKS_PER_SECOND
        } else {
            ms.saturating_mul(1_000_000)
        };
        Self(monotonic_ns().saturating_add(duration))
    }

    #[inline]
    pub fn expired(self) -> bool {
        monotonic_ns() >= self.0
    }

    pub fn tick(self) -> u64 {
        self.0.div_ceil(1_000_000_000 / TICKS_PER_SECOND)
    }
}

/// Bootloader UTC epoch plus monotonic timebase; absence is explicit.
pub fn realtime() -> Option<(u64, u64)> {
    let epoch = BOOT_EPOCH_SECONDS.load(Ordering::Relaxed);
    if epoch < 0 {
        return None;
    }
    let ns = monotonic_ns();
    Some((
        (epoch as u64).checked_add(ns / 1_000_000_000)?,
        ns % 1_000_000_000,
    ))
}

pub fn is_high_resolution() -> bool {
    hpet::resolution_ns().is_some()
}
#[no_mangle]
pub extern "C" fn kernel_clock_ns() -> u64 {
    monotonic_ns()
}

#[cfg(feature = "boot-smoke-tests")]
pub fn run_clock_probe() {
    if !is_high_resolution() {
        crate::memory::serial_write("[CLOCK-PROBE] PIT fallback active\r\n");
        return;
    }
    let irq = crate::sync::InterruptGuard::new();
    let ticks = MONOTONIC_TICKS.load(Ordering::Relaxed);
    let start = monotonic_ns();
    let mut end = start;
    for _ in 0..10_000_000 {
        end = monotonic_ns();
        assert!(end >= start, "clock regressed");
        if end - start >= 2_000_000 {
            break;
        }
        core::hint::spin_loop();
    }
    assert!(
        end - start >= 2_000_000,
        "clock did not progress with IRQ disabled"
    );
    assert_eq!(MONOTONIC_TICKS.load(Ordering::Relaxed), ticks);
    drop(irq);
    crate::memory::serial_write(
        "[CLOCK-PROBE] IRQ-off counter progress >=2000000 ns, PIT delivery unchanged OK\r\n",
    );
}
