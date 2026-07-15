use genos_abi::{BootInfo, MemoryRegionKind};

static mut TOTAL_USABLE: u64 = 0;
static mut NEXT_FRAME: u64 = 0;
static mut LAST_FRAME: u64 = 0;

pub fn init(boot_info: &BootInfo) {
    let mut total = 0;
    let mut first = 0;
    let mut last = 0;
    for region in boot_info
        .memory_map
        .regions
        .iter()
        .take(boot_info.memory_map.region_count as usize)
    {
        if region.kind == MemoryRegionKind::Usable {
            total += region.size;
            if first == 0 {
                first = align_up(region.start, 4096);
            }
            last = region.start + region.size;
        }
    }
    unsafe {
        TOTAL_USABLE = total;
        NEXT_FRAME = first;
        LAST_FRAME = last;
    }
    crate::serial::print("Usable memory bytes: ");
    crate::serial::print_u64(total);
    crate::serial::println("");
}

pub fn usable_bytes() -> u64 {
    unsafe { TOTAL_USABLE }
}

pub fn alloc_frame() -> Option<u64> {
    unsafe {
        if NEXT_FRAME == 0 || NEXT_FRAME + 4096 > LAST_FRAME {
            None
        } else {
            let frame = NEXT_FRAME;
            NEXT_FRAME += 4096;
            Some(frame)
        }
    }
}

const fn align_up(value: u64, align: u64) -> u64 {
    (value + align - 1) & !(align - 1)
}
