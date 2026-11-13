//! Isolated physical allocator test on BSP/APs. No heap, mappings or task entry.
use super::{get_pmm, PhysicalAddress, PhysicalMemoryManager};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
const WIDTH: usize = 4;
const ROUNDS: usize = 128;
const CAPACITY: usize = crate::cpu::CPU_CAPACITY;
static STARTED: AtomicBool = AtomicBool::new(false);
static FAILED: AtomicBool = AtomicBool::new(false);
static ABORT: AtomicBool = AtomicBool::new(false);
static CPUS: AtomicUsize = AtomicUsize::new(0);
static ARRIVED: AtomicUsize = AtomicUsize::new(0);
static PHASE: AtomicUsize = AtomicUsize::new(0);
static LIVE: [AtomicUsize; CAPACITY * WIDTH] = [const { AtomicUsize::new(0) }; CAPACITY * WIDTH];
static COMPLETED: [AtomicUsize; CAPACITY] = [const { AtomicUsize::new(0) }; CAPACITY];
extern "C" {
    fn hal_mp_count() -> u64;
    fn hal_mp_run_pmm_probe() -> i32;
}

// Never called under the PMM lock. Bounded failure releases every participant
// rather than stranding the others if one CPU fails to reach a phase.
fn barrier() -> bool {
    if ABORT.load(Ordering::Acquire) {
        return false;
    }
    let phase = PHASE.load(Ordering::Acquire);
    if ARRIVED.fetch_add(1, Ordering::AcqRel) + 1 == CPUS.load(Ordering::Relaxed) {
        ARRIVED.store(0, Ordering::Relaxed);
        PHASE.fetch_add(1, Ordering::Release);
        return true;
    }
    for _ in 0..500_000_000usize {
        if ABORT.load(Ordering::Acquire) {
            return false;
        }
        if PHASE.load(Ordering::Acquire) != phase {
            return true;
        }
        core::hint::spin_loop();
    }
    FAILED.store(true, Ordering::Release);
    ABORT.store(true, Ordering::Release);
    false
}

fn release(
    pmm: Option<&PhysicalMemoryManager>,
    slot: usize,
    frames: &[Option<PhysicalAddress>; WIDTH],
) {
    for (index, frame) in frames.iter().enumerate() {
        LIVE[slot * WIDTH + index].store(0, Ordering::Release);
        if let (Some(pmm), Some(frame)) = (pmm, frame) {
            pmm.free_frame(*frame);
        }
    }
}

#[no_mangle]
pub extern "C" fn kernel_pmm_probe_worker(slot: u64) {
    let slot = slot as usize;
    if slot >= CPUS.load(Ordering::Acquire) || slot != crate::cpu::index() {
        FAILED.store(true, Ordering::Release);
        ABORT.store(true, Ordering::Release);
        return;
    }
    let pmm = get_pmm();
    for round in 0..ROUNDS {
        let mut frames = [None; WIDTH];
        if let Some(pmm) = pmm {
            if round % 2 == 0 {
                if let Some(first) = pmm.alloc_contiguous(WIDTH) {
                    for (index, frame) in frames.iter_mut().enumerate() {
                        *frame = Some(PhysicalAddress(first.0 + index * super::PAGE_SIZE));
                    }
                }
            } else {
                for frame in &mut frames {
                    *frame = pmm.alloc_frame();
                }
            }
        }
        for (index, frame) in frames.iter().enumerate() {
            let encoded = frame.map(|frame| frame.0 + 1).unwrap_or(0);
            if encoded == 0 {
                FAILED.store(true, Ordering::Release);
            }
            LIVE[slot * WIDTH + index].store(encoded, Ordering::Release);
        }
        // All CPUs hold allocations simultaneously before uniqueness is read.
        if !barrier() {
            release(pmm, slot, &frames);
            break;
        }
        if slot == 0 {
            let limit = CPUS.load(Ordering::Relaxed) * WIDTH;
            for index in 0..limit {
                let address = LIVE[index].load(Ordering::Acquire);
                for other in index + 1..limit {
                    if address != 0 && address == LIVE[other].load(Ordering::Acquire) {
                        FAILED.store(true, Ordering::Release);
                    }
                }
            }
        }
        if !barrier() {
            release(pmm, slot, &frames);
            break;
        }
        release(pmm, slot, &frames);
        if !barrier() {
            break;
        }
        COMPLETED[slot].fetch_add(1, Ordering::Release);
    }
}

pub fn run_concurrent_smoke() -> bool {
    if STARTED.swap(true, Ordering::AcqRel) {
        return false;
    }
    let Some(pmm) = get_pmm() else {
        return false;
    };
    let count = unsafe { hal_mp_count() } as usize;
    if count == 0 || count > CAPACITY || crate::cpu::index() != 0 {
        return false;
    }
    let before = pmm.stats_bytes();
    CPUS.store(count, Ordering::Release);
    if unsafe { hal_mp_run_pmm_probe() } == 0
        || FAILED.load(Ordering::Acquire)
        || before != pmm.stats_bytes()
    {
        return false;
    }
    for (cpu, completed) in COMPLETED.iter().enumerate().take(count) {
        if completed.load(Ordering::Acquire) != ROUNDS {
            return false;
        }
        let mut line = crate::FmtBuf::new();
        line.push_str("[PMM-SMP] cpu=");
        line.push_u64(cpu as u64);
        line.push_str(" rounds=");
        line.push_u64(ROUNDS as u64);
        line.push_str("\r\n");
        super::serial_write(line.as_str());
    }
    let mut line = crate::FmtBuf::new();
    line.push_str("[PMM-SMP] cpus=");
    line.push_u64(count as u64);
    line.push_str(" rounds=");
    line.push_u64(ROUNDS as u64);
    line.push_str(" frames=");
    line.push_u64((count * ROUNDS * WIDTH) as u64);
    line.push_str(" unique/reclaimed OK\r\n");
    super::serial_write(line.as_str());
    true
}
