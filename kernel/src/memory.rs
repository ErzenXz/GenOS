use genos_abi::{BootInfo, MemoryRegionKind};
use kernel::physmem::{scrub_page, AllocatorStats, FrameAllocator, KERNEL_BITMAP_WORDS, PAGE_SIZE};

static mut ALLOCATOR: FrameAllocator<KERNEL_BITMAP_WORDS> = FrameAllocator::new();
#[cfg(feature = "memory-test-faults")]
static mut FAIL_AFTER: Option<usize> = None;

pub fn init(boot_info: &BootInfo) {
    // SAFETY: BSP initialization with interrupts disabled is the only owner.
    // Initialize in static storage to avoid copying the 256-KiB bitmap.
    let allocator = unsafe { &mut *core::ptr::addr_of_mut!(ALLOCATOR) };
    for region in boot_info
        .memory_map
        .regions
        .iter()
        .take(boot_info.memory_map.region_count as usize)
    {
        if region.kind == MemoryRegionKind::Usable && !allocator.add_region(*region) {
            crate::serial::println("FRAME_ALLOCATOR_MAP_REJECTED overlap_or_capacity=true");
            crate::arch::halt_loop();
        }
    }
    let total = allocator.usable_bytes();
    let regions = allocator.region_count();
    crate::serial::print("Usable memory bytes: ");
    crate::serial::print_u64(total);
    crate::serial::print(" regions=");
    crate::serial::print_u64(regions as u64);
    crate::serial::println("");
    assert!(allocator.is_consistent());
    crate::serial::println("FRAME_ALLOCATOR_BITMAP_READY capacity_gib=8");
}

/// All allocator access is confined to an IRQ-masked BSP critical section.
/// The closure cannot return an allocator borrow. NMI/fatal handlers must not
/// call this module; the boot admission gate excludes other CPUs.
fn with_allocator<R>(operation: impl FnOnce(&mut FrameAllocator<KERNEL_BITMAP_WORDS>) -> R) -> R {
    crate::arch::without_interrupts(|| {
        // SAFETY: the admitted BSP is the sole owner. Local IRQs are masked
        // before creating this exclusive reference and no borrow escapes it.
        operation(unsafe { &mut *core::ptr::addr_of_mut!(ALLOCATOR) })
    })
}

#[allow(dead_code)]
pub fn usable_bytes() -> u64 {
    with_allocator(|allocator| allocator.usable_bytes())
}

/// Return an exclusively granted, zero-filled identity-mapped physical frame.
pub fn alloc_frame() -> Option<u64> {
    with_allocator(|allocator| {
        #[cfg(feature = "memory-test-faults")]
        // SAFETY: the same IRQ-masked BSP section protects fault configuration.
        unsafe {
            if let Some(remaining) = *core::ptr::addr_of!(FAIL_AFTER) {
                if remaining == 0 {
                    return None;
                }
                core::ptr::addr_of_mut!(FAIL_AFTER).write(Some(remaining - 1));
            }
        }
        let frame = allocator.alloc_frame()?;
        // SAFETY: the bitmap just granted this usable, aligned page exclusively.
        // Firmware/retained supervisor identity mappings cover managed RAM.
        // It is zeroed before any caller can publish a user or table mapping.
        unsafe {
            core::ptr::write_bytes(frame as *mut u8, 0, PAGE_SIZE as usize);
        }
        Some(frame)
    })
}

#[cfg(feature = "memory-test-faults")]
pub fn fail_after(allocations: Option<usize>) {
    crate::arch::without_interrupts(|| {
        // SAFETY: fault configuration shares the allocator's BSP/IRQ ownership.
        unsafe {
            core::ptr::addr_of_mut!(FAIL_AFTER).write(allocations);
        }
    });
}

/// Callers must retire mappings/references before returning a grant. Validation
/// precedes all stores; the bitmap remains allocated until scrubbing completes.
pub fn free_frame(frame: u64) -> bool {
    with_allocator(|allocator| {
        allocator.free_frame_with(frame, |frame| {
            // SAFETY: the live-grant check established aligned managed RAM before
            // this callback. The caller retired all users, the BSP owns this page,
            // IRQs are masked, and supervisor identity mappings still cover it.
            let page = unsafe { &mut *(frame as *mut [u8; PAGE_SIZE as usize]) };
            scrub_page(page);
            true
        })
    })
}

pub fn allocated_frames() -> u64 {
    with_allocator(|allocator| allocator.allocated_frames())
}

#[allow(dead_code)]
pub fn recycled_frames() -> usize {
    with_allocator(|allocator| allocator.recycled_frames())
}

pub fn snapshot() -> (AllocatorStats, bool) {
    with_allocator(|allocator| (allocator.stats(), allocator.is_consistent()))
}

pub struct Report {
    bytes: [u8; 512],
    len: usize,
}
impl core::fmt::Write for Report {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        let end = self.len.checked_add(text.len()).ok_or(core::fmt::Error)?;
        if end > self.bytes.len() {
            return Err(core::fmt::Error);
        }
        self.bytes[self.len..end].copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
}
impl Report {
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

pub fn report() -> Result<Report, core::fmt::Error> {
    let (stats, consistent) = snapshot();
    let mut report = Report {
        bytes: [0; 512],
        len: 0,
    };
    stats.write_report(&mut report, consistent)?;
    Ok(report)
}

#[cfg(feature = "memory-test-faults")]
pub fn run_hygiene_probe() -> bool {
    crate::arch::without_interrupts(|| {
        let baseline = allocated_frames();
        let Some(frame) = alloc_frame() else {
            return false;
        };
        // SAFETY: validation owns the exact grant and its physical identity map.
        // IRQs stay masked throughout poisoning, release, inspection and reuse.
        // The post-release reads deliberately inspect the free page before any
        // allocator caller can acquire it, proving release-time scrubbing.
        let (initial_zero, poison_intact, erased) = unsafe {
            let initial_zero = (0..PAGE_SIZE)
                .all(|offset| core::ptr::read_volatile((frame + offset) as *const u8) == 0);
            for offset in 0..PAGE_SIZE {
                core::ptr::write_volatile((frame + offset) as *mut u8, 0xa5);
            }
            let denied = !free_frame(frame + 1) && !free_frame(0);
            let poison_intact = denied
                && (0..PAGE_SIZE)
                    .all(|offset| core::ptr::read_volatile((frame + offset) as *const u8) == 0xa5);
            let erased = free_frame(frame)
                && (0..PAGE_SIZE)
                    .all(|offset| core::ptr::read_volatile((frame + offset) as *const u8) == 0);
            (initial_zero, poison_intact, erased)
        };
        let double_denied = !free_frame(frame);
        let Some(reused) = alloc_frame() else {
            return false;
        };
        // SAFETY: the new live grant remains exclusively owned by this probe.
        let reuse_zero = unsafe {
            (0..PAGE_SIZE)
                .all(|offset| core::ptr::read_volatile((reused + offset) as *const u8) == 0)
        };
        let returned = free_frame(reused);
        initial_zero
            && poison_intact
            && erased
            && double_denied
            && reused == frame
            && reuse_zero
            && returned
            && allocated_frames() == baseline
            && snapshot().1
    })
}
