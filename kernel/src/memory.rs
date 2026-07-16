use genos_abi::{BootInfo, MemoryRegionKind};
use kernel::physmem::FrameAllocator;

static mut ALLOCATOR: FrameAllocator = FrameAllocator::new();

pub fn init(boot_info: &BootInfo) {
    let mut allocator = FrameAllocator::new();
    for region in boot_info
        .memory_map
        .regions
        .iter()
        .take(boot_info.memory_map.region_count as usize)
    {
        if region.kind == MemoryRegionKind::Usable {
            allocator.add_region(*region);
        }
    }
    let total = allocator.usable_bytes();
    let regions = allocator.region_count();
    unsafe {
        core::ptr::addr_of_mut!(ALLOCATOR).write(allocator);
    }
    crate::serial::print("Usable memory bytes: ");
    crate::serial::print_u64(total);
    crate::serial::print(" regions=");
    crate::serial::print_u64(regions as u64);
    crate::serial::println("");
}

pub fn usable_bytes() -> u64 {
    unsafe { (*core::ptr::addr_of!(ALLOCATOR)).usable_bytes() }
}

pub fn alloc_frame() -> Option<u64> {
    unsafe { (*core::ptr::addr_of_mut!(ALLOCATOR)).alloc_frame() }
}

pub fn free_frame(frame: u64) -> bool {
    unsafe { (*core::ptr::addr_of_mut!(ALLOCATOR)).free_frame(frame) }
}

pub fn allocated_frames() -> u64 {
    unsafe { (*core::ptr::addr_of!(ALLOCATOR)).allocated_frames() }
}

pub fn recycled_frames() -> usize {
    unsafe { (*core::ptr::addr_of!(ALLOCATOR)).recycled_frames() }
}
