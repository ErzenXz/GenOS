use genos_abi::{BootInfo, MemoryRegionKind};
use kernel::physmem::{FrameAllocator, KERNEL_BITMAP_WORDS};

static mut ALLOCATOR: FrameAllocator<KERNEL_BITMAP_WORDS> = FrameAllocator::new();
#[cfg(feature = "memory-test-faults")]
static mut FAIL_AFTER: Option<usize> = None;

pub fn init(boot_info: &BootInfo) {
    // SAFETY: BSP initialization with interrupts disabled is the only owner.
    // Initialize in static storage: the bitmap is larger than the boot stack.
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

#[allow(dead_code)]
// Retained for deferred diagnostics and allocator telemetry.
pub fn usable_bytes() -> u64 {
    unsafe { (*core::ptr::addr_of!(ALLOCATOR)).usable_bytes() }
}

pub fn alloc_frame() -> Option<u64> {
    #[cfg(feature = "memory-test-faults")]
    // SAFETY: validation probe runs with IF clear, before untrusted execution.
    unsafe {
        if let Some(remaining) = *core::ptr::addr_of!(FAIL_AFTER) {
            if remaining == 0 {
                return None;
            }
            core::ptr::addr_of_mut!(FAIL_AFTER).write(Some(remaining - 1));
        }
    }
    unsafe { (*core::ptr::addr_of_mut!(ALLOCATOR)).alloc_frame() }
}

#[cfg(feature = "memory-test-faults")]
pub fn fail_after(allocations: Option<usize>) {
    // SAFETY: only the single-core validation probe calls this with IF clear.
    unsafe {
        core::ptr::addr_of_mut!(FAIL_AFTER).write(allocations);
    }
}

pub fn free_frame(frame: u64) -> bool {
    unsafe { (*core::ptr::addr_of_mut!(ALLOCATOR)).free_frame(frame) }
}

pub fn allocated_frames() -> u64 {
    unsafe { (*core::ptr::addr_of!(ALLOCATOR)).allocated_frames() }
}

#[allow(dead_code)]
// Retained for serial diagnostics of returned, reusable frame grants.
pub fn recycled_frames() -> usize {
    unsafe { (*core::ptr::addr_of!(ALLOCATOR)).recycled_frames() }
}
