//! Kernel heap: a BSS bootstrap region, then contiguous PMM blocks through HHDM.
//! The IRQ-safe lock protects the free list; headers retain alignment padding
//! and small tails so deallocation can return the complete allocation.

use core::alloc::{GlobalAlloc, Layout};
#[cfg(not(test))]
use core::cell::UnsafeCell;
use core::ptr;

use crate::memory::pmm::PhysicalAddress;
use crate::sync::IrqSafeSpinLock;

const PAGE_SIZE: usize = 4096;

// Minimum physical block requested when the free list cannot satisfy a layout.
const HEAP_GROW_CHUNK: usize = 1024 * 1024;

struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

#[repr(C)]
struct AllocationHeader {
    block_start: usize,
    block_size: usize,
}

// Addresses are used only under the owning allocator's lock.
struct HeapInner {
    free_list: usize,
    total_bytes: usize,
}

impl HeapInner {
    const fn new() -> Self {
        Self {
            free_list: 0,
            total_bytes: 0,
        }
    }

    fn add_bootstrap(&mut self, heap_start: usize, heap_size: usize) {
        let initial_block = heap_start as *mut FreeBlock;
        unsafe {
            (*initial_block).size = heap_size;
            (*initial_block).next = ptr::null_mut();
        }
        self.free_list = initial_block as usize;
        self.total_bytes = heap_size;
    }

    /// Insert by address and coalesce adjacent free regions.
    unsafe fn add_free_region(&mut self, start: usize, size: usize) {
        let block = start as *mut FreeBlock;
        (*block).size = size;

        let mut current = self.free_list as *mut FreeBlock;
        let mut previous: *mut FreeBlock = ptr::null_mut();
        while !current.is_null() && (current as usize) < start {
            previous = current;
            current = (*current).next;
        }

        (*block).next = current;
        if previous.is_null() {
            self.free_list = block as usize;
        } else {
            (*previous).next = block;
        }

        // Merge the following region before checking the preceding region.
        let next = (*block).next;
        if !next.is_null() && start + size == next as usize {
            (*block).size += (*next).size;
            (*block).next = (*next).next;
        }
        if !previous.is_null() && (previous as usize) + (*previous).size == block as usize {
            (*previous).size += (*block).size;
            (*previous).next = (*block).next;
        }
    }

    unsafe fn alloc_from_free_list(&mut self, layout: Layout) -> *mut u8 {
        let payload_size = layout.size().max(1);
        let user_align = layout
            .align()
            .max(core::mem::align_of::<AllocationHeader>());
        let header_size = core::mem::size_of::<AllocationHeader>();
        let minimum_free = core::mem::size_of::<FreeBlock>();

        let mut current_ptr = self.free_list as *mut FreeBlock;
        let mut prev_ptr: *mut FreeBlock = ptr::null_mut();

        while !current_ptr.is_null() {
            let current = &mut *current_ptr;

            let block_start = current_ptr as usize;
            let block_end = match block_start.checked_add(current.size) {
                Some(end) => end,
                None => return ptr::null_mut(),
            };
            let user_addr = align_up(block_start + header_size, user_align);
            let requested_end = match user_addr.checked_add(payload_size) {
                Some(end) => align_up(end, core::mem::align_of::<FreeBlock>()),
                None => return ptr::null_mut(),
            };

            if requested_end <= block_end {
                let remaining_size = block_end - requested_end;
                let (allocated_end, replacement) = if remaining_size >= minimum_free {
                    let next = requested_end as *mut FreeBlock;
                    (*next).size = remaining_size;
                    (*next).next = current.next;
                    (requested_end, next)
                } else {
                    // A tail too small for FreeBlock belongs to this allocation.
                    (block_end, current.next)
                };

                if prev_ptr.is_null() {
                    self.free_list = replacement as usize;
                } else {
                    (*prev_ptr).next = replacement;
                }

                let header = (user_addr - header_size) as *mut AllocationHeader;
                (*header).block_start = block_start;
                (*header).block_size = allocated_end - block_start;
                return user_addr as *mut u8;
            }

            prev_ptr = current_ptr;
            current_ptr = current.next;
        }

        ptr::null_mut()
    }

    unsafe fn dealloc(&mut self, ptr: *mut u8) {
        let header =
            (ptr as usize - core::mem::size_of::<AllocationHeader>()) as *const AllocationHeader;
        let block_start = (*header).block_start;
        let size = (*header).block_size;
        self.add_free_region(block_start, size);
    }

    fn stats(&self) -> KernelHeapStats {
        let total = self.total_bytes;
        let mut free = 0usize;
        let mut blocks = 0usize;
        let mut current_ptr = self.free_list as *const FreeBlock;
        unsafe {
            while !current_ptr.is_null() {
                free = free.saturating_add((*current_ptr).size);
                blocks += 1;
                current_ptr = (*current_ptr).next as *const FreeBlock;
            }
        }
        KernelHeapStats {
            total_bytes: total as u64,
            free_bytes: free as u64,
            used_bytes: total.saturating_sub(free) as u64,
            free_blocks: blocks as u64,
        }
    }
}

/// Acquired PMM backing that has not been published to the heap.
struct HeapRegion {
    physical: PhysicalAddress,
    virtual_start: usize,
    size: usize,
}

impl HeapRegion {
    fn acquire(min_bytes: usize) -> Option<Self> {
        let size = align_up_checked(min_bytes.max(HEAP_GROW_CHUNK), PAGE_SIZE)?;
        let physical = crate::memory::pmm::get_pmm()?.alloc_contiguous(size / PAGE_SIZE)?;
        let virtual_start = crate::memory::vmm::phys_to_virt(physical.as_usize());
        let region = Self { physical, virtual_start, size };
        if virtual_start == 0 {
            region.release();
            return None;
        }
        Some(region)
    }

    fn release(self) {
        let pmm = crate::memory::pmm::get_pmm().expect("unpublished heap backing owns PMM frames");
        for offset in (0..self.size).step_by(PAGE_SIZE) {
            pmm.free_frame(PhysicalAddress::from_usize(self.physical.as_usize() + offset));
        }
    }
}

pub struct KernelAllocator {
    inner: IrqSafeSpinLock<HeapInner>,
    growth: IrqSafeSpinLock<()>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct KernelHeapStats {
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub used_bytes: u64,
    pub free_blocks: u64,
}

impl KernelAllocator {
    pub const fn new() -> Self {
        Self {
            inner: IrqSafeSpinLock::new(HeapInner::new()),
            growth: IrqSafeSpinLock::new(()),
        }
    }

    pub fn init(&self, heap_start: usize, heap_size: usize) {
        self.inner.lock().add_bootstrap(heap_start, heap_size);
    }

    pub fn stats(&self) -> KernelHeapStats {
        self.inner.lock().stats()
    }
}

unsafe impl GlobalAlloc for KernelAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let result = self.inner.lock().alloc_from_free_list(layout);
        if !result.is_null() {
            return result;
        }

        // Coordinate only callers that exhaust the free list. They recheck
        // after any earlier growth publishes its frames, before asking the PMM.
        let _growth = self.growth.lock();
        let result = self.inner.lock().alloc_from_free_list(layout);
        if !result.is_null() {
            return result;
        }

        // A contiguous PMM search must not hold the heap lock. A free can
        // satisfy this allocation while the search runs.
        let required = layout
            .size()
            .saturating_add(layout.align())
            .saturating_add(core::mem::size_of::<AllocationHeader>())
            .saturating_add(core::mem::size_of::<FreeBlock>());
        let growth = HeapRegion::acquire(required);
        let mut inner = self.inner.lock();
        let result = inner.alloc_from_free_list(layout);
        if !result.is_null() {
            drop(inner);
            if let Some(region) = growth {
                region.release();
            }
            return result;
        }
        let Some(region) = growth else {
            return ptr::null_mut();
        };
        inner.add_free_region(region.virtual_start, region.size);
        inner.total_bytes += region.size;
        inner.alloc_from_free_list(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        if ptr.is_null() {
            return;
        }
        self.inner.lock().dealloc(ptr);
    }
}

fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

fn align_up_checked(value: usize, align: usize) -> Option<usize> {
    value.checked_add(align - 1).map(|v| v & !(align - 1))
}

#[cfg(not(test))]
#[global_allocator]
static ALLOCATOR: KernelAllocator = KernelAllocator::new();

#[cfg(not(test))]
pub fn init_heap(heap_start: usize, heap_size: usize) {
    ALLOCATOR.init(heap_start, heap_size);
}

// Boot initializes the allocator with this buffer once, using raw pointers.
#[repr(align(4096))]
#[cfg(not(test))]
struct AlignedHeap(UnsafeCell<[u8; 2 * 1024 * 1024]>);
#[cfg(not(test))]
unsafe impl Sync for AlignedHeap {}

#[cfg(not(test))]
static KERNEL_HEAP: AlignedHeap = AlignedHeap(UnsafeCell::new([0; 2 * 1024 * 1024]));

#[cfg(not(test))]
pub fn init() {
    crate::memory::serial_write("[HEAP] START\r\n");

    let heap_start = KERNEL_HEAP.0.get() as *const u8 as usize;
    let heap_size = core::mem::size_of::<[u8; 2 * 1024 * 1024]>();
    init_heap(heap_start, heap_size);

    crate::memory::serial_write("[HEAP] OK (PMM-backed, growable)\r\n");
}

#[cfg(not(test))]
pub fn heap_stats() -> KernelHeapStats {
    ALLOCATOR.stats()
}

#[cfg(test)]
pub fn heap_stats() -> KernelHeapStats {
    KernelHeapStats::default()
}
