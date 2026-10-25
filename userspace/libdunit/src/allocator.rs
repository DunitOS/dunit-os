//! Native heap allocation. Host-side pure SDK tests use the host allocator.
use crate::{syscall5, SYSCALL_MMAP};
use core::alloc::{GlobalAlloc, Layout};
use core::ptr::{self, null_mut};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const PAGE_SIZE: usize = 4096;
const HEAP_GROW_CHUNK: usize = 64 * 1024;
const PROT_READ: usize = 1 << 0;
const PROT_WRITE: usize = 1 << 1;
const MAP_PRIVATE: usize = 1 << 1;
const MAP_ANONYMOUS: usize = 1 << 5;

#[repr(C)]
struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

#[repr(C)]
struct AllocationHeader {
    block_start: usize,
    block_size: usize,
}

struct RuntimeAllocator {
    free_list: AtomicUsize,
    locked: AtomicBool,
}

impl RuntimeAllocator {
    const fn new() -> Self {
        Self {
            free_list: AtomicUsize::new(0),
            locked: AtomicBool::new(false),
        }
    }

    fn lock(&self) {
        while self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
    }

    fn unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }

    unsafe fn allocate_from_free_list(&self, layout: Layout) -> *mut u8 {
        let payload_size = layout.size().max(1);
        let header_size = core::mem::size_of::<AllocationHeader>();
        let user_align = layout
            .align()
            .max(core::mem::align_of::<AllocationHeader>());
        let minimum_free = core::mem::size_of::<FreeBlock>();
        let mut current_ptr = self.free_list.load(Ordering::Relaxed) as *mut FreeBlock;
        let mut previous: *mut FreeBlock = ptr::null_mut();

        while !current_ptr.is_null() {
            let block_start = current_ptr as usize;
            let block_end = block_start.saturating_add((*current_ptr).size);
            let user_addr = align_up(block_start + header_size, user_align);
            let requested_end = align_up(
                user_addr.saturating_add(payload_size),
                core::mem::align_of::<FreeBlock>(),
            );

            if requested_end <= block_end {
                let remaining = block_end - requested_end;
                let (allocated_end, replacement) = if remaining >= minimum_free {
                    let next = requested_end as *mut FreeBlock;
                    (*next).size = remaining;
                    (*next).next = (*current_ptr).next;
                    (requested_end, next)
                } else {
                    (block_end, (*current_ptr).next)
                };

                if previous.is_null() {
                    self.free_list
                        .store(replacement as usize, Ordering::Relaxed);
                } else {
                    (*previous).next = replacement;
                }

                let header = (user_addr - header_size) as *mut AllocationHeader;
                (*header).block_start = block_start;
                (*header).block_size = allocated_end - block_start;
                return user_addr as *mut u8;
            }

            previous = current_ptr;
            current_ptr = (*current_ptr).next;
        }

        null_mut()
    }

    unsafe fn add_free_region(&self, start: usize, size: usize) {
        let block = start as *mut FreeBlock;
        (*block).size = size;

        let mut current = self.free_list.load(Ordering::Relaxed) as *mut FreeBlock;
        let mut previous: *mut FreeBlock = ptr::null_mut();
        while !current.is_null() && (current as usize) < start {
            previous = current;
            current = (*current).next;
        }

        (*block).next = current;
        if previous.is_null() {
            self.free_list.store(block as usize, Ordering::Relaxed);
        } else {
            (*previous).next = block;
        }
        coalesce_free_neighbors(self, previous, block);
    }

    unsafe fn grow(&self, minimum: usize) -> bool {
        let requested = align_up(minimum.max(HEAP_GROW_CHUNK), PAGE_SIZE);
        let mapped = syscall5(
            SYSCALL_MMAP,
            0,
            requested,
            PROT_READ | PROT_WRITE,
            MAP_PRIVATE | MAP_ANONYMOUS,
            0,
        );
        if mapped < 0 {
            return false;
        }
        self.add_free_region(mapped as usize, requested);
        true
    }
}

unsafe fn coalesce_free_neighbors(
    allocator: &RuntimeAllocator,
    previous: *mut FreeBlock,
    mut block: *mut FreeBlock,
) {
    if !previous.is_null() && previous as usize + (*previous).size == block as usize {
        (*previous).size += (*block).size;
        (*previous).next = (*block).next;
        block = previous;
    }

    let next = (*block).next;
    if !next.is_null() && block as usize + (*block).size == next as usize {
        (*block).size += (*next).size;
        (*block).next = (*next).next;
    }

    if previous.is_null() {
        allocator.free_list.store(block as usize, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for RuntimeAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.lock();
        let mut result = self.allocate_from_free_list(layout);
        if result.is_null() {
            let required = layout
                .size()
                .saturating_add(layout.align())
                .saturating_add(core::mem::size_of::<AllocationHeader>());
            if self.grow(required) {
                result = self.allocate_from_free_list(layout);
            }
        }
        self.unlock();
        result
    }

    unsafe fn dealloc(&self, pointer: *mut u8, _layout: Layout) {
        if pointer.is_null() {
            return;
        }
        self.lock();
        let header = (pointer as usize - core::mem::size_of::<AllocationHeader>())
            as *const AllocationHeader;
        self.add_free_region((*header).block_start, (*header).block_size);
        self.unlock();
    }
}

#[global_allocator]
static ALLOCATOR: RuntimeAllocator = RuntimeAllocator::new();

fn align_up(value: usize, align: usize) -> usize {
    value.saturating_add(align - 1) & !(align - 1)
}
