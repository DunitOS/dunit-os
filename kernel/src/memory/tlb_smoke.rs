//! Isolated AP memory test: real CR3/PTE/TLB use with ordinary IRQs masked.
//! APs allocate nothing, access no tasks/VFS and return to their parked roots.
use super::{pmm, tlb, vmm};
use crate::sync::IrqSafeSpinLock;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
const ROUNDS: usize = 32;
const VIRT: usize = 0x40000000;
const A: u64 = 0x13579bdf2468ace0;
const B: u64 = 0xfedcba9876543210;
static SPACE: AtomicUsize = AtomicUsize::new(0);
static ROOT: AtomicUsize = AtomicUsize::new(0);
static KERNEL_ALIAS: AtomicUsize = AtomicUsize::new(0);
static FRAMES: [AtomicUsize; 2] = [const { AtomicUsize::new(0) }; 2];
static CPUS: AtomicUsize = AtomicUsize::new(0);
static COMMAND: AtomicUsize = AtomicUsize::new(0);
static STOP: AtomicBool = AtomicBool::new(false);
static FAILED: AtomicBool = AtomicBool::new(false);
#[repr(align(64))]
struct Slot {
    ready: AtomicBool,
    step: AtomicUsize,
    left: AtomicUsize,
    value: AtomicU64,
    kernel_value: AtomicU64,
}
impl Slot {
    const fn new() -> Self {
        Self {
            ready: AtomicBool::new(false),
            step: AtomicUsize::new(0),
            left: AtomicUsize::new(0),
            value: AtomicU64::new(0),
            kernel_value: AtomicU64::new(0),
        }
    }
}
static SLOTS: [Slot; crate::cpu::CPU_CAPACITY] = [const { Slot::new() }; crate::cpu::CPU_CAPACITY];
fn wait_until(mut predicate: impl FnMut() -> bool) -> bool {
    let deadline = crate::clock::monotonic_ns().saturating_add(2_000_000_000);
    for _ in 0..100_000_000usize {
        if predicate() {
            return true;
        }
        if STOP.load(Ordering::Acquire) || crate::clock::monotonic_ns() >= deadline {
            break;
        }
        core::hint::spin_loop();
    }
    FAILED.store(true, Ordering::Release);
    STOP.store(true, Ordering::Release);
    false
}
#[no_mangle]
pub extern "C" fn kernel_vm_probe_worker(cpu: u64) {
    let cpu = cpu as usize;
    if cpu != crate::cpu::index() || cpu >= CPUS.load(Ordering::Acquire) {
        FAILED.store(true, Ordering::Release);
        STOP.store(true, Ordering::Release);
        return;
    }
    if cpu != 0 {
        if crate::sync::interrupts_enabled() {
            FAILED.store(true, Ordering::Release);
            STOP.store(true, Ordering::Release);
            return;
        }
        let previous = unsafe { vmm::active_root_frame() };
        let space = unsafe {
            &*(SPACE.load(Ordering::Acquire) as *const IrqSafeSpinLock<vmm::AddressSpace>)
        };
        {
            let _mapping = space.lock();
            unsafe {
                vmm::switch_to_root_frame(ROOT.load(Ordering::Acquire));
            }
        }
        SLOTS[cpu].value.store(
            unsafe { core::ptr::read_volatile(VIRT as *const u64) },
            Ordering::Relaxed,
        );
        SLOTS[cpu].kernel_value.store(
            unsafe { core::ptr::read_volatile(KERNEL_ALIAS.load(Ordering::Relaxed) as *const u64) },
            Ordering::Relaxed,
        );
        SLOTS[cpu].ready.store(true, Ordering::Release);
        for round in 1..=ROUNDS {
            // Alternate entered-root shootdown with leave/reenter boundaries.
            // Entry takes the same AS lock as production dispatch; the CR3
            // load must see current PTEs even if this CPU missed the mask.
            if round % 2 == 0 {
                unsafe {
                    vmm::switch_to_root_frame(previous);
                }
                SLOTS[cpu].left.store(round, Ordering::Release);
            }
            if !wait_until(|| COMMAND.load(Ordering::Acquire) >= round) {
                break;
            }
            if round % 2 == 0 {
                let _mapping = space.lock();
                unsafe {
                    vmm::switch_to_root_frame(ROOT.load(Ordering::Acquire));
                }
            }
            let value = unsafe { core::ptr::read_volatile(VIRT as *const u64) };
            SLOTS[cpu].value.store(value, Ordering::Relaxed);
            SLOTS[cpu].kernel_value.store(
                unsafe {
                    core::ptr::read_volatile(KERNEL_ALIAS.load(Ordering::Relaxed) as *const u64)
                },
                Ordering::Relaxed,
            );
            SLOTS[cpu].step.store(round, Ordering::Release);
        }
        // The address-space owner stays pinned by the BSP until the C
        // rendezvous confirms this restoration and the worker's return.
        unsafe {
            vmm::switch_to_root_frame(previous);
        }
        return;
    }
    if !wait_until(|| {
        (1..CPUS.load(Ordering::Relaxed)).all(|i| SLOTS[i].ready.load(Ordering::Acquire))
    }) {
        return;
    }
    if (1..CPUS.load(Ordering::Relaxed)).any(|i| {
        SLOTS[i].value.load(Ordering::Relaxed) != A
            || SLOTS[i].kernel_value.load(Ordering::Relaxed) != A
    }) {
        FAILED.store(true, Ordering::Release);
        STOP.store(true, Ordering::Release);
        return;
    }
    let space =
        unsafe { &*(SPACE.load(Ordering::Acquire) as *const IrqSafeSpinLock<vmm::AddressSpace>) };
    for round in 1..=ROUNDS {
        if round % 2 == 0 {
            if !wait_until(|| {
                (1..CPUS.load(Ordering::Relaxed))
                    .all(|i| SLOTS[i].left.load(Ordering::Acquire) == round)
            }) {
                break;
            }
            assert_eq!(
                tlb::active_mask(ROOT.load(Ordering::Acquire)),
                0,
                "all probe CPUs left before retarget"
            );
        }
        let frame = FRAMES[round % 2].load(Ordering::Relaxed);
        let changed = (|| {
            let mut space = space.lock();
            if !space
                .protect_user_page(vmm::VirtualAddress(VIRT), vmm::PageFlags::NO_EXECUTE, true)
                .ok()?
            {
                return None;
            }
            if !space.unmap_user_page(vmm::VirtualAddress(VIRT)).ok()? {
                return None;
            }
            space
                .map_user_frame(
                    vmm::VirtualAddress(VIRT),
                    pmm::PhysicalAddress(frame),
                    vmm::PageFlags::WRITABLE | vmm::PageFlags::NO_EXECUTE,
                )
                .ok()?;
            Some(())
        })();
        if changed.is_none() {
            FAILED.store(true, Ordering::Release);
            STOP.store(true, Ordering::Release);
            break;
        }
        let alias = KERNEL_ALIAS.load(Ordering::Relaxed);
        assert!(
            vmm::unmap_mmio_region(alias, 4096),
            "probe kernel alias unmap"
        );
        assert_eq!(
            vmm::map_firmware_region(frame, 4096),
            Some(alias),
            "probe kernel alias reuse"
        );
        COMMAND.store(round, Ordering::Release);
        if !wait_until(|| {
            (1..CPUS.load(Ordering::Relaxed))
                .all(|i| SLOTS[i].step.load(Ordering::Acquire) == round)
        }) {
            break;
        }
        let expected = if round % 2 == 0 { A } else { B };
        if (1..CPUS.load(Ordering::Relaxed)).any(|i| {
            SLOTS[i].value.load(Ordering::Relaxed) != expected
                || SLOTS[i].kernel_value.load(Ordering::Relaxed) != expected
        }) {
            FAILED.store(true, Ordering::Release);
            STOP.store(true, Ordering::Release);
            break;
        }
    }
    STOP.store(true, Ordering::Release);
}
pub fn run_tlb_smoke() -> bool {
    extern "C" {
        fn hal_mp_count() -> u64;
        fn hal_mp_run_vm_probe() -> i32;
    }
    let count = unsafe { hal_mp_count() } as usize;
    CPUS.store(count, Ordering::Release);
    let pmm = pmm::get_pmm().expect("TLB probe PMM");
    let first = pmm.alloc_frame().expect("TLB probe frame A");
    let second = pmm.alloc_frame().expect("TLB probe frame B");
    unsafe {
        core::ptr::write_volatile(vmm::phys_to_virt(first.0) as *mut u64, A);
        core::ptr::write_volatile(vmm::phys_to_virt(second.0) as *mut u64, B);
    }
    FRAMES[0].store(first.0, Ordering::Relaxed);
    FRAMES[1].store(second.0, Ordering::Relaxed);
    let mut space = vmm::AddressSpace::new().expect("TLB probe AS");
    space
        .map_user_frame(
            vmm::VirtualAddress(VIRT),
            first,
            vmm::PageFlags::WRITABLE | vmm::PageFlags::NO_EXECUTE,
        )
        .expect("TLB probe first map");
    let tables = space.page_table_frame_count();
    let space = Arc::new(IrqSafeSpinLock::new(space));
    KERNEL_ALIAS.store(
        vmm::map_firmware_region(first.0, 4096).expect("TLB kernel alias"),
        Ordering::Release,
    );
    ROOT.store(space.lock().root_frame().0, Ordering::Release);
    SPACE.store(Arc::as_ptr(&space) as usize, Ordering::Release);
    let before = tlb::remote_acks();
    assert_ne!(
        unsafe { hal_mp_run_vm_probe() },
        0,
        "AP VM rendezvous must finish before reclaim"
    );
    let passed = !FAILED.load(Ordering::Acquire)
        && (1..count).all(|i| SLOTS[i].step.load(Ordering::Acquire) == ROUNDS)
        && tlb::remote_acks() - before >= ((count - 1) * (ROUNDS + 2 * (ROUNDS / 2))) as u64;
    assert!(vmm::unmap_mmio_region(
        KERNEL_ALIAS.load(Ordering::Relaxed),
        4096
    ));
    SPACE.store(0, Ordering::Release);
    let (_, free_before) = pmm::stats_bytes();
    drop(space); // active mask must be empty before root/table frames are freed
    pmm.free_frame(first);
    pmm.free_frame(second);
    let (_, free_after) = pmm::stats_bytes();
    if passed && free_after - free_before == ((tables + 2) * 4096) as u64 {
        let mut line = crate::FmtBuf::new();
        line.push_str("[TLB-SMP] cpus=");
        line.push_u64(count as u64);
        line.push_str(" rounds=32 AP retarget, IRQ-off NMI acks and final reclaim OK\r\n");
        crate::memory::serial_write(line.as_str());
        true
    } else {
        crate::memory::serial_write("[TLB-SMP] FAIL\r\n");
        false
    }
}
