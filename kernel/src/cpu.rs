/// Enable x87/SSE for Rust code compiled with SSE instructions.
pub unsafe fn init_fpu() {
    let mut cr0: u64;
    core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack));
    cr0 &= !(1 << 3); // TS: no longer "task switched"
    cr0 &= !(1 << 2); // EM: no x87 emulation
    cr0 |= 1 << 1; // MP: monitor coprocessor
    core::arch::asm!("mov cr0, {}", in(reg) cr0, options(nomem, nostack));

    let mut cr4: u64;
    core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
    cr4 &= !(1 << 16); // No user WRGSBASE: GS anchors kernel CPU-local state.
    cr4 |= 1 << 9; // OSFXSR: enable FXSAVE/FXRSTOR and SSE
    cr4 |= 1 << 10; // OSXMMEXCPT: enable SIMD FP exceptions
    core::arch::asm!("mov cr4, {}", in(reg) cr4, options(nomem, nostack));

    core::arch::asm!("fninit", options(nostack, preserves_flags));
}

include!("cpu_layout.rs");
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU64};
#[repr(align(64))]
pub struct Runtime {
    pub current_pid: AtomicU64,
    pub exit_requested: AtomicBool,
    pub yield_requested: AtomicBool,
    pub schedule_hint: AtomicU64,
    pub exit_code: AtomicI32,
    pub exit_kind: AtomicI32,
    pub preempt_switch_requested: AtomicBool,
}
impl Runtime {
    const fn new() -> Self {
        Self {
            current_pid: AtomicU64::new(0),
            exit_requested: AtomicBool::new(false),
            yield_requested: AtomicBool::new(false),
            schedule_hint: AtomicU64::new(0),
            exit_code: AtomicI32::new(0),
            exit_kind: AtomicI32::new(0),
            preempt_switch_requested: AtomicBool::new(false),
        }
    }
}
static RUNTIME: [Runtime; CPU_CAPACITY] = [const { Runtime::new() }; CPU_CAPACITY];
pub fn index() -> usize {
    unsafe {
        extern "C" {
            fn hal_cpu_index() -> u64;
        }
        hal_cpu_index() as usize
    }
}
pub fn runtime() -> &'static Runtime {
    &RUNTIME[index()]
}

extern "C" {
    fn hal_mp_apic_physical() -> u64;
    fn hal_mp_prepare(mapped: u64) -> i32;
    fn hal_mp_start() -> i32;
    fn hal_mp_count() -> u64;
    fn hal_mp_snapshot(index: u64, out: *mut u64);
}
#[no_mangle]
pub extern "C" fn kernel_mp_ticks() -> u64 {
    crate::clock::monotonic_ticks()
}
pub fn prepare_multiprocessor() {
    let phys = unsafe { hal_mp_apic_physical() };
    let mapped = if phys != 0 {
        crate::memory::vmm::map_mmio_region(phys as usize, 4096).expect("local APIC mapping") as u64
    } else {
        0
    };
    assert_ne!(unsafe { hal_mp_prepare(mapped) }, 0, "CPU-local/APIC setup");
}
pub fn start_multiprocessor() {
    let result = unsafe { hal_mp_start() };
    let count = unsafe { hal_mp_count() };
    for i in 0..count {
        let mut state = [0u64; 5];
        unsafe { hal_mp_snapshot(i, state.as_mut_ptr()) };
        let mut line = crate::FmtBuf::new();
        line.push_str("[SMP] cpu=");
        line.push_u64(i);
        line.push_str(" apic=");
        line.push_u64(state[0]);
        line.push_str(" ready=");
        line.push_u64(state[1]);
        line.push_str(" ipi=");
        line.push_u64(state[2]);
        line.push_str(" timer=");
        line.push_u64(state[3]);
        line.push_str(" work=");
        line.push_u64(state[4]);
        line.push_str("\r\n");
        crate::memory::serial_write(line.as_str());
    }
    assert_ne!(result, 0, "AP startup/timer/IPI handshake");
    let mut line = crate::FmtBuf::new();
    line.push_str("[SMP] hardware_online=");
    line.push_u64(count);
    line.push_str(" scheduler=up dispatch_cpus=1 APs=parked\r\n");
    crate::memory::serial_write(line.as_str());
}

/// Permanent per-online-CPU storage allocated on the BSP before AP publication.
/// AP paths never allocate, and parked CPU stacks cannot be reclaimed.
#[no_mangle]
pub extern "C" fn kernel_cpu_alloc(size: u64, alignment: u64) -> u64 {
    if size == 0 {
        return 0;
    }
    let Ok(layout) = core::alloc::Layout::from_size_align(size as usize, alignment as usize) else {
        return 0;
    };
    unsafe { alloc::alloc::alloc_zeroed(layout) as u64 }
}
