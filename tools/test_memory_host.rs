//! Actual kernel heap and ELF parser. Only IRQ control and physical backing
//! are host adapters; heap allocation, growth, rollback and locking are real.
#[allow(dead_code)]
#[path = "../kernel/src/sync.rs"]
mod actual_sync;
mod sync {
    use crate::actual_sync::{SpinLock, SpinLockGuard};
    use std::ops::{Deref, DerefMut};

    std::thread_local! {
        pub static HEAP_HELD: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }
    pub struct IrqSafeSpinLock<T>(SpinLock<T>);
    pub struct Guard<'a, T>(SpinLockGuard<'a, T>);

    impl<T> IrqSafeSpinLock<T> {
        pub const fn new(value: T) -> Self {
            Self(SpinLock::new(value))
        }
        pub fn lock(&self) -> Guard<'_, T> {
            let guard = self.0.lock();
            // The heap's free-list lock contains state. Its separate growth
            // coordinator has a zero-sized payload.
            if std::mem::size_of::<T>() != 0 {
                HEAP_HELD.with(|depth| depth.set(depth.get() + 1));
            }
            Guard(guard)
        }
    }
    impl<T> Deref for Guard<'_, T> {
        type Target = T;
        fn deref(&self) -> &T {
            &self.0
        }
    }
    impl<T> DerefMut for Guard<'_, T> {
        fn deref_mut(&mut self) -> &mut T {
            &mut self.0
        }
    }
    impl<T> Drop for Guard<'_, T> {
        fn drop(&mut self) {
            if std::mem::size_of::<T>() != 0 {
                HEAP_HELD.with(|depth| depth.set(depth.get() - 1));
            }
        }
    }
}
#[allow(dead_code)]
#[path = "../kernel/src/allocator.rs"]
mod allocator;
#[allow(dead_code)]
#[path = "../kernel/src/elf/parser.rs"]
mod elf;

use std::alloc::{alloc_zeroed, dealloc, GlobalAlloc, Layout};
use std::cell::RefCell;
use std::collections::BTreeSet;

#[derive(Default)]
struct Backing {
    regions: Vec<(usize, Layout)>,
    live: BTreeSet<usize>,
    available: usize,
    fail_mapping: bool,
}
impl Drop for Backing {
    fn drop(&mut self) {
        for (address, layout) in self.regions.drain(..) {
            unsafe { dealloc(address as *mut u8, layout) };
        }
    }
}
thread_local! {
    static BACKING: RefCell<Backing> = RefCell::new(Backing::default());
    static ON_GROW: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
}
mod memory {
    pub mod pmm {
        #[derive(Clone, Copy)]
        pub struct PhysicalAddress(usize);
        impl PhysicalAddress {
            pub fn from_usize(address: usize) -> Self {
                Self(address)
            }
            pub fn as_usize(self) -> usize {
                self.0
            }
        }
        pub struct Pmm;
        pub fn get_pmm() -> Option<&'static Pmm> {
            Some(&Pmm)
        }
        impl Pmm {
            pub fn alloc_contiguous(&self, pages: usize) -> Option<PhysicalAddress> {
                assert_eq!(crate::sync::HEAP_HELD.with(std::cell::Cell::get), 0);
                if let Some(hook) = crate::ON_GROW.with(|hook| hook.borrow_mut().take()) {
                    hook();
                }
                crate::BACKING.with(|pool| {
                    let mut pool = pool.borrow_mut();
                    if pages > pool.available {
                        return None;
                    }
                    let layout = std::alloc::Layout::from_size_align(pages * 4096, 4096).unwrap();
                    let address = unsafe { std::alloc::alloc_zeroed(layout) } as usize;
                    assert_ne!(address, 0);
                    pool.regions.push((address, layout));
                    for page in 0..pages {
                        assert!(pool.live.insert(address + page * 4096));
                    }
                    pool.available -= pages;
                    Some(PhysicalAddress(address))
                })
            }
            pub fn free_frame(&self, frame: PhysicalAddress) {
                assert_eq!(crate::sync::HEAP_HELD.with(std::cell::Cell::get), 0);
                crate::BACKING.with(|pool| {
                    let mut pool = pool.borrow_mut();
                    assert!(
                        pool.live.remove(&frame.0),
                        "frame freed twice or outside allocation"
                    );
                    pool.available += 1;
                });
            }
        }
    }
    pub mod vmm {
        pub fn phys_to_virt(physical: usize) -> usize {
            crate::BACKING.with(|pool| {
                if pool.borrow().fail_mapping {
                    0
                } else {
                    physical
                }
            })
        }
    }
}

struct Heap {
    allocator: allocator::KernelAllocator,
    address: usize,
    layout: Layout,
}
impl Heap {
    fn new(bytes: usize) -> Self {
        let layout = Layout::from_size_align(bytes, 4096).unwrap();
        let address = unsafe { alloc_zeroed(layout) } as usize;
        assert_ne!(address, 0);
        let allocator = allocator::KernelAllocator::new();
        allocator.init(address, bytes);
        Self {
            allocator,
            address,
            layout,
        }
    }
    fn assert_reclaimed(&self) {
        let stats = self.allocator.stats();
        assert_eq!(stats.free_bytes, stats.total_bytes);
        assert_eq!(stats.used_bytes, 0);
        assert_eq!(stats.free_blocks, 1);
    }
}
impl Drop for Heap {
    fn drop(&mut self) {
        unsafe { dealloc(self.address as *mut u8, self.layout) };
    }
}

#[test]
fn mixed_sizes_alignments_and_reverse_free_reclaim_the_real_heap() {
    let heap = Heap::new(65536);
    for align in [1, 2, 8, 16, 64, 256, 4096] {
        let mut allocations = Vec::new();
        for size in [0, 1, 7, 8, 15, 31, 63, 127, 255, 511, 1023] {
            let layout = Layout::from_size_align(size, align).unwrap();
            let ptr = unsafe { heap.allocator.alloc(layout) };
            assert!(!ptr.is_null());
            assert_eq!(ptr as usize % align, 0);
            assert!(ptr as usize >= heap.address);
            assert!(ptr as usize + size <= heap.address + heap.layout.size());
            unsafe { ptr.write_bytes(size as u8, size) };
            allocations.push((ptr, layout, size as u8));
        }
        for (ptr, layout, byte) in allocations.into_iter().rev() {
            assert!(unsafe { std::slice::from_raw_parts(ptr, layout.size()) }
                .iter()
                .all(|&b| b == byte));
            unsafe { heap.allocator.dealloc(ptr, layout) };
        }
        heap.assert_reclaimed();
    }
}

#[test]
fn exhaustion_and_small_tail_preserve_accounting_and_reuse() {
    let heap = Heap::new(4096);
    let layout = Layout::from_size_align(4096 - 24, 8).unwrap();
    let ptr = unsafe { heap.allocator.alloc(layout) };
    assert!(!ptr.is_null());
    assert_eq!(heap.allocator.stats().free_bytes, 0);
    assert!(unsafe { heap.allocator.alloc(Layout::new::<u64>()) }.is_null());
    unsafe { heap.allocator.dealloc(ptr, layout) };
    heap.assert_reclaimed();
    let impossible = Layout::from_size_align(65536, 4096).unwrap();
    assert!(unsafe { heap.allocator.alloc(impossible) }.is_null());
    heap.assert_reclaimed();
}

#[test]
fn concurrent_allocations_do_not_overlap_or_lose_free_blocks() {
    let heap = Heap::new(1 << 20);
    std::thread::scope(|scope| {
        for tag in 1..=8u8 {
            let allocator = &heap.allocator;
            scope.spawn(move || {
                for round in 0..2000 {
                    let layout =
                        Layout::from_size_align(1 + round % 1024, 1 << (round % 9)).unwrap();
                    let ptr = unsafe { allocator.alloc(layout) };
                    assert!(!ptr.is_null());
                    unsafe { ptr.write_bytes(tag, layout.size()) };
                    std::thread::yield_now();
                    assert!(unsafe { std::slice::from_raw_parts(ptr, layout.size()) }
                        .iter()
                        .all(|&b| b == tag));
                    unsafe { allocator.dealloc(ptr, layout) };
                }
            });
        }
    });
    heap.assert_reclaimed();
}

#[test]
fn growth_and_reuse_keep_the_same_physical_backing() {
    BACKING.with(|pool| pool.borrow_mut().available = 256);
    let allocator = allocator::KernelAllocator::new();
    let layout = Layout::from_size_align(8192, 4096).unwrap();
    let ptr = unsafe { allocator.alloc(layout) };
    assert!(!ptr.is_null());
    unsafe {
        ptr.write_bytes(0xa5, layout.size());
        allocator.dealloc(ptr, layout)
    };
    let stats = allocator.stats();
    assert_eq!(stats.total_bytes, 1 << 20);
    assert_eq!(stats.free_bytes, stats.total_bytes);
    let again = unsafe { allocator.alloc(layout) };
    assert_eq!(ptr, again);
    unsafe { allocator.dealloc(again, layout) };
    BACKING.with(|pool| {
        let pool = pool.borrow();
        assert_eq!(pool.regions.len(), 1);
        assert_eq!(pool.live.len(), 256);
        assert_eq!(pool.available, 0);
    });
}

#[test]
fn growth_retries_a_concurrent_free_and_returns_unpublished_backing() {
    use std::sync::Arc;

    let heap = Arc::new(Heap::new(4096));
    let layout = Layout::from_size_align(4096 - 24, 8).unwrap();
    let first = unsafe { heap.allocator.alloc(layout) };
    assert!(!first.is_null());
    assert_eq!(heap.allocator.stats().free_bytes, 0);
    let address = first as usize;
    let hook_heap = Arc::clone(&heap);
    ON_GROW.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || unsafe {
            hook_heap.allocator.dealloc(address as *mut u8, layout);
        }));
    });
    BACKING.with(|pool| pool.borrow_mut().available = 256);
    let next = unsafe { heap.allocator.alloc(layout) };
    assert_eq!(next, first);
    assert_eq!(heap.allocator.stats().total_bytes, 4096);
    BACKING.with(|pool| {
        let pool = pool.borrow();
        assert_eq!(pool.regions.len(), 1);
        assert_eq!(pool.available, 256);
        assert!(pool.live.is_empty());
    });
    unsafe { heap.allocator.dealloc(next, layout) };
    heap.assert_reclaimed();
}

#[test]
fn failed_growth_mapping_returns_every_acquired_frame() {
    BACKING.with(|pool| {
        let mut pool = pool.borrow_mut();
        pool.available = 256;
        pool.fail_mapping = true;
    });
    let allocator = allocator::KernelAllocator::new();
    assert!(unsafe { allocator.alloc(Layout::new::<u64>()) }.is_null());
    assert_eq!(allocator.stats().total_bytes, 0);
    BACKING.with(|pool| {
        let pool = pool.borrow();
        assert_eq!(pool.available, 256);
        assert!(pool.live.is_empty());
    });
}
