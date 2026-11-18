//! Feature-only isolated AP worker: atomics/APIC only, never task dispatch.
use core::sync::atomic::{AtomicU64, Ordering};
static GENERATION: AtomicU64 = AtomicU64::new(0);
static PHASE: AtomicU64 = AtomicU64::new(0);
static PROBE_LOCK: crate::sync::IrqSafeSpinLock<()> = crate::sync::IrqSafeSpinLock::new(());
const RESULT: i64 = 0x1357_2468;

#[no_mangle]
pub extern "C" fn kernel_irq_progress_worker(cpu: u64) {
    if cpu != 1 || PHASE.load(Ordering::Acquire) != 1 { return; }
    let generation = super::request(0).expect("BSP reschedule request admission");
    GENERATION.store(generation, Ordering::Release);
}

pub(crate) fn debug_log() -> i64 {
    assert_eq!(crate::cpu::index(), 0);
    assert!(crate::sync::interrupts_enabled(), "syscall body must permit IRQs");
    match PHASE.load(Ordering::Acquire) {
        0 => {
            extern "C" { fn hal_mp_irq_probe() -> i32; }
            {
                let _lock = PROBE_LOCK.lock();
                assert!(!crate::sync::interrupts_enabled(), "task lock must mask IRQs");
                let start = crate::clock::pit_interrupts();
                PHASE.store(1, Ordering::Release);
                assert_ne!(unsafe { hal_mp_irq_probe() }, 0, "IRQ probe requires AP");
                let mut ready = false;
                for _ in 0..100_000_000 {
                    if GENERATION.load(Ordering::Acquire) != 0 { ready = true; break; }
                    core::hint::spin_loop();
                }
                assert!(ready, "AP ordinary IPI send timeout");
                assert_eq!(super::pending(), None, "masked IPI must remain pending in APIC");
                assert_eq!(crate::clock::pit_interrupts(), start, "PIT masked by lock");
            }
            assert!(crate::sync::interrupts_enabled());
            let start = crate::clock::pit_interrupts();
            let mut progressed = false;
            for _ in 0..200_000_000 {
                if super::pending().is_some() && crate::clock::pit_interrupts() - start >= 3 {
                    progressed = true; break;
                }
                core::hint::spin_loop();
            }
            assert!(progressed, "PIT/IPI progress inside syscall body");
            let generation = GENERATION.load(Ordering::Acquire);
            assert!(!super::acknowledged(0, generation), "delivery is not HAL return");
            PHASE.store(2, Ordering::Release);
            RESULT
        }
        2 => {
            let generation = GENERATION.load(Ordering::Acquire);
            assert!(super::acknowledged(0, generation), "ACK requires HAL/FPU/TLS return");
            assert!(PROBE_LOCK.try_lock().is_some(), "no mutex across HAL return");
            PHASE.store(3, Ordering::Release);
            crate::memory::serial_write("[IRQ-PROGRESS] PIT and AP IPI inside syscall; lock masks IRQ; HAL return ACK OK\r\n");
            RESULT
        }
        _ => -22,
    }
}
