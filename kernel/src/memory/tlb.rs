//! Acknowledged invalidation. Receivers acquire no locks and run on private
//! NMI/IST stacks, so IRQ-off lock waiters cannot deadlock shootdown senders.
use crate::cpu::{index, CPU_CAPACITY};
use crate::sync::IrqSafeSpinLock;
use core::sync::atomic::{fence, AtomicU64, AtomicUsize, Ordering};
static ONLINE: AtomicUsize = AtomicUsize::new(1);
static ROOTS: [AtomicUsize; CPU_CAPACITY] = [const { AtomicUsize::new(0) }; CPU_CAPACITY];
#[repr(align(64))]
struct Mailbox {
    root: AtomicUsize,
    page: AtomicUsize,
    request: AtomicU64,
    ack: AtomicU64,
}
impl Mailbox {
    const fn new() -> Self {
        Self {
            root: AtomicUsize::new(0),
            page: AtomicUsize::new(0),
            request: AtomicU64::new(0),
            ack: AtomicU64::new(0),
        }
    }
}
static MAILBOXES: [Mailbox; CPU_CAPACITY] = [const { Mailbox::new() }; CPU_CAPACITY];
// One sender owns all recipient mailboxes until their exact ticket is acked.
// Receiver never acquires this or an AddressSpace/MMIO/task/allocator lock.
static SEND: IrqSafeSpinLock<u64> = IrqSafeSpinLock::new(0);
#[cfg(feature = "boot-smoke-tests")]
static REMOTE_ACKS: AtomicU64 = AtomicU64::new(0);
pub(crate) fn online(count: usize) {
    assert!((1..=CPU_CAPACITY).contains(&count));
    ONLINE.store(count, Ordering::Release);
}
pub(super) fn publish_root(root: usize) {
    ROOTS[index()].store(root, Ordering::SeqCst);
}
#[no_mangle]
pub extern "C" fn kernel_cpu_track_root(cpu: u64, root: u64) {
    assert_eq!(cpu as usize, index());
    publish_root(root as usize);
}
pub(super) fn active_mask(root: usize) -> u64 {
    let mut mask = 0;
    for cpu in 0..ONLINE.load(Ordering::Acquire) {
        if ROOTS[cpu].load(Ordering::SeqCst) == root {
            mask |= 1u64 << cpu;
        }
    }
    mask
}
pub(super) fn invalidate(root: usize, page: usize) {
    invalidate_inner(root, page, false);
}
pub(super) fn invalidate_kernel(page: usize) {
    invalidate_inner(0, page, true);
}
fn invalidate_inner(root: usize, page: usize, kernel: bool) {
    // Store -> snapshot ordering pairs with publication BEFORE MOV CR3. A CPU
    // missed by this mask must subsequently load CR3 and observe the new PTE.
    fence(Ordering::SeqCst);
    let targets = if kernel {
        u64::MAX >> (u64::BITS as usize - ONLINE.load(Ordering::Acquire))
    } else {
        active_mask(root)
    };
    let targets = targets & !(1u64 << index());
    if targets == 0 {
        return;
    }
    let mut sequence = SEND.lock();
    *sequence = sequence.checked_add(1).expect("TLB generation exhausted");
    let ticket = *sequence;
    extern "C" {
        fn hal_mp_send_nmi(cpu: u64) -> i32;
    }
    for cpu in 0..ONLINE.load(Ordering::Acquire) {
        if targets & (1u64 << cpu) == 0 {
            continue;
        }
        let mailbox = &MAILBOXES[cpu];
        mailbox
            .root
            .store(if kernel { 0 } else { root }, Ordering::Relaxed);
        mailbox.page.store(page, Ordering::Relaxed);
        mailbox.request.store(ticket, Ordering::Release);
        // ICR/WRMSR delivery must not outrun the mailbox stores, including
        // non-serializing x2APIC MSRs on processors that require this fence.
        fence(Ordering::SeqCst);
        assert_ne!(
            unsafe { hal_mp_send_nmi(cpu as u64) },
            0,
            "TLB NMI delivery rejected"
        );
    }
    let deadline = crate::clock::monotonic_ns().saturating_add(2_000_000_000);
    for attempt in 0..100_000_000usize {
        let mut complete = true;
        for cpu in 0..ONLINE.load(Ordering::Acquire) {
            if targets & (1u64 << cpu) != 0 && MAILBOXES[cpu].ack.load(Ordering::Acquire) != ticket
            {
                complete = false;
            }
        }
        if complete {
            return;
        }
        assert!(
            attempt < 99_999_999 && crate::clock::monotonic_ns() < deadline,
            "TLB acknowledgement timeout; memory must not be reused"
        );
        core::hint::spin_loop();
    }
}
/// Called only after the common ISR established kernel GS and saved SIMD on
/// the private NMI stack. Never escape a syscall or touch task/AS metadata.
pub(crate) fn handle_nmi() -> bool {
    let mailbox = &MAILBOXES[index()];
    let ticket = mailbox.request.load(Ordering::Acquire);
    if ticket == mailbox.ack.load(Ordering::Relaxed) {
        return false;
    }
    let root = mailbox.root.load(Ordering::Relaxed);
    let page = mailbox.page.load(Ordering::Relaxed);
    unsafe {
        super::vmm::invalidate_if_active(root, page);
    }
    #[cfg(feature = "boot-smoke-tests")]
    REMOTE_ACKS.fetch_add(1, Ordering::Relaxed);
    mailbox.ack.store(ticket, Ordering::Release);
    true
}
#[cfg(feature = "boot-smoke-tests")]
pub(super) fn remote_acks() -> u64 {
    REMOTE_ACKS.load(Ordering::Acquire)
}
