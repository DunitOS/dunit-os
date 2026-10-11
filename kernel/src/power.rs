//! Power control mechanisms: machine reset (reboot) and power-off.
//!
//! Kernel = mechanism. The shell `reboot`/`poweroff` commands (policy) call
//! these; the kernel only knows HOW to pull the reset/power lines, not WHEN.
//!
//! Power-off targets the ACPI "soft off" ports exposed by the common emulators
//! (QEMU, Bochs, VirtualBox) plus a couple of hypervisor variants. Full ACPI
//! power-off on real hardware needs the FADT's PM1a_CNT port and the `\_S5`
//! sleep type parsed from the ACPI tables — a later milestone. Reset uses the
//! standard 0xCF9 reset-control register and the i8042 keyboard-controller CPU
//! reset pulse, both of which work on real hardware and the emulators.
//!
//! Each function writes a `[POWER] ... requested` marker to the serial log
//! BEFORE pulling the line, then settles briefly; if control returns the
//! platform did not honour the request and a `FAILED` marker is emitted. This
//! makes "did it actually stop the machine?" observable to `tools/qemu_test.py`
//! (a working power-off exits QEMU during the settle window, so FAILED never
//! prints).

use crate::hal;

/// Spin long enough for a virtual machine to tear down after a power/reset
/// request before we conclude the request was not honoured. Pure busy-wait: by
/// this point interrupts are off and there is nothing else to do.
fn settle() {
    for _ in 0..200_000_000u64 {
        core::hint::spin_loop();
    }
}

/// Power the machine off. Returns only if no known mechanism is supported
/// (e.g. real hardware without the emulator ACPI ports).
pub fn poweroff() {
    crate::memory::serial_write("[POWER] poweroff requested\r\n");
    crate::sync::disable_interrupts();
    unsafe {
        hal::hal_outw(0x604, 0x2000); // QEMU >= 2.0 ACPI PM1a_CNT (SLP_EN|S5)
        hal::hal_outw(0xB004, 0x2000); // Bochs / older QEMU
        hal::hal_outw(0x4004, 0x3400); // VirtualBox
        hal::hal_outw(0x600, 0x2000); // Cloud Hypervisor / misc
    }
    settle();
    // Still running: the platform did not honour the request. Restore interrupts
    // so the shell stays usable, then report the failure.
    crate::sync::enable_interrupts();
    crate::memory::serial_write("[POWER] poweroff FAILED (unsupported platform)\r\n");
}

/// Reset (reboot) the machine. Returns only if no reset line is honoured.
pub fn reboot() {
    crate::memory::serial_write("[POWER] reboot requested\r\n");
    crate::sync::disable_interrupts();
    unsafe {
        // 0xCF9 reset control: RST_CPU (0x04) with full-reset (0x08) around the
        // SYS_RST pulse (0x02).
        hal::hal_outb(0xCF9, 0x02);
        hal::hal_outb(0xCF9, 0x06);
        hal::hal_outb(0xCF9, 0x0E);
        // i8042: wait for the input buffer to drain, then pulse the CPU reset.
        for _ in 0..100_000u32 {
            if (hal::hal_inb(0x64) & 0x02) == 0 {
                break;
            }
        }
        hal::hal_outb(0x64, 0xFE);
    }
    settle();
    crate::sync::enable_interrupts();
    crate::memory::serial_write("[POWER] reboot FAILED (unsupported platform)\r\n");
}
