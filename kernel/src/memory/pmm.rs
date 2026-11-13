use core::cell::UnsafeCell;

use super::serial_write;

const PAGE_SIZE: usize = 4096;
const MEMMAP_USABLE: u32 = 0;
const MAX_REGIONS: usize = 32;
const BITMAP_BYTES: usize = 65536;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MemRegion {
    pub base: u64,
    pub length: u64,
    pub region_type: u32,
    pub _pad: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalAddress(pub usize);

impl PhysicalAddress {
    pub fn as_usize(&self) -> usize {
        self.0
    }

    pub fn from_usize(addr: usize) -> Self {
        Self(addr)
    }
}

// Runtime lock order: heap / AddressSpace / device owner -> frame pool.
// The frame-pool lock is a leaf: never allocate heap, map pages, log, call a
// process callback or wait for another CPU while holding it.
struct FramePool {
    bitmap: &'static mut [u8],
    usable_frames: usize,
    free_frames: usize,
}
impl FramePool {
    fn mark_free(&mut self, index: usize) -> bool {
        let mask = 1u8 << (index % 8);
        let byte = &mut self.bitmap[index / 8];
        if *byte & mask == 0 {
            return false;
        }
        *byte &= !mask;
        true
    }
}

pub struct PhysicalMemoryManager {
    pool: crate::sync::IrqSafeSpinLock<FramePool>,
    total_frames: usize,
    base_addr: usize,
}

impl PhysicalMemoryManager {
    pub fn new(memory_start: usize, memory_size: usize, bitmap: &'static mut [u8]) -> Self {
        let total_frames = memory_size / PAGE_SIZE;
        let bitmap_size = (total_frames + 7) / 8;
        assert_eq!(memory_start % PAGE_SIZE, 0, "unaligned physical pool");
        assert!(
            memory_start.checked_add(total_frames * PAGE_SIZE).is_some(),
            "physical pool overflow"
        );
        assert!(
            bitmap_size <= bitmap.len(),
            "physical pool bitmap too small"
        );
        let bitmap = &mut bitmap[..bitmap_size];
        bitmap.fill(0);
        // Tail bits never name frames and must not appear allocatable.
        if total_frames % 8 != 0 {
            bitmap[bitmap_size - 1] = !((1u16 << (total_frames % 8)) - 1) as u8;
        }
        Self {
            pool: crate::sync::IrqSafeSpinLock::new(FramePool {
                bitmap,
                usable_frames: total_frames,
                free_frames: total_frames,
            }),
            total_frames,
            base_addr: memory_start,
        }
    }

    fn reserve_all_frames(&self) {
        let mut pool = self.pool.lock();
        pool.bitmap.fill(0xff);
        pool.usable_frames = 0;
        pool.free_frames = 0;
    }

    /// Makes only complete pages inside a usable memory-map region allocatable.
    fn mark_region_free_in_pool(&self, base: u64, length: u64) -> usize {
        let region_start = base as usize;
        let region_end = region_start.saturating_add(length as usize);
        let aligned_start = region_start.saturating_add(PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let aligned_end = region_end & !(PAGE_SIZE - 1);
        let pool_end = self.base_addr + self.total_frames * PAGE_SIZE;
        let start = aligned_start.max(self.base_addr);
        let end = aligned_end.min(pool_end);
        if start >= end {
            return 0;
        }
        let first = (start - self.base_addr) / PAGE_SIZE;
        let last = (end - self.base_addr) / PAGE_SIZE;
        let mut pool = self.pool.lock();
        let mut newly_free = 0;
        for index in first..last {
            if pool.mark_free(index) {
                newly_free += 1;
            }
        }
        pool.usable_frames += newly_free;
        pool.free_frames += newly_free;
        newly_free
    }

    pub fn alloc_frame(&self) -> Option<PhysicalAddress> {
        let mut pool = self.pool.lock();
        let byte_index = pool.bitmap.iter().position(|byte| *byte != 0xff)?;
        let bit = (!pool.bitmap[byte_index]).trailing_zeros() as usize;
        let index = byte_index * 8 + bit;
        debug_assert!(index < self.total_frames);
        pool.bitmap[byte_index] |= 1 << bit;
        pool.free_frames -= 1;
        Some(PhysicalAddress(self.base_addr + index * PAGE_SIZE))
    }

    /// First-fit contiguous run; scan and reservation form one transaction.
    pub fn alloc_contiguous(&self, count: usize) -> Option<PhysicalAddress> {
        if count == 0 || count > self.total_frames {
            return None;
        }
        if count == 1 {
            return self.alloc_frame();
        } // no lock held on this path
        let mut pool = self.pool.lock();
        if count > pool.free_frames {
            return None;
        }
        let mut run = 0;
        for index in 0..self.total_frames {
            if pool.bitmap[index / 8] & (1 << (index % 8)) == 0 {
                run += 1;
                if run == count {
                    let start = index + 1 - count;
                    for frame in start..=index {
                        pool.bitmap[frame / 8] |= 1 << (frame % 8);
                    }
                    pool.free_frames -= count;
                    return Some(PhysicalAddress(self.base_addr + start * PAGE_SIZE));
                }
            } else {
                run = 0;
            }
        }
        None
    }

    /// Caller owns this allocated frame and has already retired all users/TLBs.
    pub fn free_frame(&self, addr: PhysicalAddress) {
        let Some(offset) = addr.0.checked_sub(self.base_addr) else {
            return;
        };
        if offset % PAGE_SIZE != 0 {
            return;
        }
        let index = offset / PAGE_SIZE;
        if index >= self.total_frames {
            return;
        }
        let mut pool = self.pool.lock();
        if pool.mark_free(index) {
            pool.free_frames += 1;
        }
    }

    pub fn available_memory(&self) -> usize {
        self.pool.lock().free_frames * PAGE_SIZE
    }

    pub fn total_memory(&self) -> usize {
        self.pool.lock().usable_frames * PAGE_SIZE
    }

    pub fn stats_bytes(&self) -> (u64, u64) {
        let pool = self.pool.lock();
        (
            (pool.usable_frames * PAGE_SIZE) as u64,
            (pool.free_frames * PAGE_SIZE) as u64,
        )
    }
}

/// Static backing storage for the physical-frame bitmap. Wrapped in an
/// `UnsafeCell` newtype instead of `static mut` so there is no `&'static mut`
/// aliasing UB: `init` takes a single mutable slice out of it exactly once and
/// hands ownership to the `PhysicalMemoryManager`, which is then published
/// read-only through `PMM_INSTANCE`.
struct BitmapStorage(UnsafeCell<[u8; BITMAP_BYTES]>);
unsafe impl Sync for BitmapStorage {}
static PMM_BITMAP: BitmapStorage = BitmapStorage(UnsafeCell::new([0; BITMAP_BYTES]));

/// Write-once singleton: initialised during `init`, only read afterwards via
/// `get_pmm`. The bitmap and accounting share one IRQ-safe leaf lock; a shared
/// manager reference never exposes its mutable storage.
static PMM_INSTANCE: crate::sync::OnceCell<PhysicalMemoryManager> = crate::sync::OnceCell::new();
// Serialize publication AND taking the one mutable static bitmap slice.
static PMM_INIT: crate::sync::IrqSafeSpinLock<()> = crate::sync::IrqSafeSpinLock::new(());

// Символы карты памяти, экспортируемые ассемблерным/C-кодом HAL до входа в Rust.
// Это `extern "C" static mut` по необходимости: их определяет и заполняет ранний
// загрузчик, Rust здесь лишь читатель. Их нельзя завернуть в `UnsafeCell`/лок,
// поэтому единственная безопасная мера — читать их ровно один раз через
// `read_volatile` в `copy_regions_from_boot` (ниже), после чего вся дальнейшая
// работа PMM идёт с локальной копией.
extern "C" {
    static mut boot_mem_regions: [MemRegion; 32];
    static mut boot_mem_region_count: u64;
}

#[inline(never)]
fn copy_regions_from_boot(cache: &mut [MemRegion; MAX_REGIONS]) -> usize {
    unsafe {
        let n = (boot_mem_region_count as usize).min(MAX_REGIONS);
        if n == 0 {
            return 0;
        }

        for i in 0..n {
            let src = &boot_mem_regions[i];
            cache[i] = MemRegion {
                base: core::ptr::read_volatile(&src.base),
                length: core::ptr::read_volatile(&src.length),
                region_type: core::ptr::read_volatile(&src.region_type),
                _pad: 0,
            };
        }
        n
    }
}

fn usable_pool_bounds(regions: &[MemRegion]) -> Option<(usize, usize)> {
    let mut first = usize::MAX;
    let mut last = 0usize;
    for region in regions {
        if region.region_type != MEMMAP_USABLE || region.length < PAGE_SIZE as u64 {
            continue;
        }
        let start = (region.base as usize).saturating_add(PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let end = (region.base as usize).saturating_add(region.length as usize) & !(PAGE_SIZE - 1);
        if start < end {
            first = first.min(start);
            last = last.max(end);
        }
    }
    (first < last).then_some((first, last))
}

pub fn init() -> bool {
    let _init = PMM_INIT.lock();
    if PMM_INSTANCE.is_initialized() {
        return true;
    }
    serial_write("[PMM] init start\r\n");
    serial_write("[PMM] scanning regions\r\n");

    let mut region_cache = [MemRegion {
        base: 0,
        length: 0,
        region_type: 0,
        _pad: 0,
    }; MAX_REGIONS];
    let copied = copy_regions_from_boot(&mut region_cache);
    if copied == 0 {
        serial_write("[PMM] FAIL\r\n");
        return false;
    }
    serial_write("[PMM] regions copied\r\n");

    let regions = &region_cache[..copied];
    let (pool_start, pool_end) = match usable_pool_bounds(regions) {
        Some(bounds) => bounds,
        None => {
            serial_write("[PMM] FAIL\r\n");
            return false;
        }
    };
    let pool_size = pool_end.saturating_sub(pool_start);
    if pool_size < PAGE_SIZE {
        serial_write("[PMM] FAIL\r\n");
        return false;
    }

    let total_frames = pool_size / PAGE_SIZE;
    let bitmap_size = (total_frames + 7) / 8;

    if bitmap_size > BITMAP_BYTES {
        serial_write("[PMM] FAIL\r\n");
        return false;
    }

    // Take the single mutable slice out of the static bitmap storage. `init`
    // runs once at boot before the PMM is published, so this is the only live
    // mutable borrow of PMM_BITMAP for the whole program.
    let bitmap = unsafe {
        let base = PMM_BITMAP.0.get() as *mut u8;
        core::slice::from_raw_parts_mut(base, bitmap_size)
    };

    serial_write("[PMM] pool ready\r\n");

    let pmm = PhysicalMemoryManager::new(pool_start, pool_size, bitmap);
    pmm.reserve_all_frames();

    serial_write("[PMM] enabling usable regions\r\n");
    for region in regions {
        if region.region_type == MEMMAP_USABLE {
            pmm.mark_region_free_in_pool(region.base, region.length);
        }
    }

    if pmm.available_memory() == 0 {
        serial_write("[PMM] FAIL: no aligned usable frames\r\n");
        return false;
    }
    serial_write("[PMM] usable regions ready\r\n");

    assert!(PMM_INSTANCE.set(pmm).is_ok());

    serial_write("[PMM] OK\r\n");
    true
}

pub fn init_pmm(memory_start: usize, memory_size: usize, bitmap: &'static mut [u8]) {
    let _init = PMM_INIT.lock();
    if PMM_INSTANCE.is_initialized() {
        return;
    }
    assert!(PMM_INSTANCE
        .set(PhysicalMemoryManager::new(
            memory_start,
            memory_size,
            bitmap,
        ))
        .is_ok());
}

pub fn get_pmm() -> Option<&'static PhysicalMemoryManager> {
    PMM_INSTANCE.get()
}

/// Returns (total_bytes, free_bytes) for the physical frame pool, or (0, 0)
/// if the PMM has not been initialized yet.
pub fn stats_bytes() -> (u64, u64) {
    match get_pmm() {
        Some(pmm) => pmm.stats_bytes(),
        None => (0, 0),
    }
}

#[cfg(feature = "boot-smoke-tests")]
#[path = "pmm_smoke.rs"]
mod smoke;
#[cfg(feature = "boot-smoke-tests")]
pub use smoke::run_concurrent_smoke;
