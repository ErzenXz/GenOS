use genos_abi::{MemoryRegion, MemoryRegionKind};

pub const PAGE_SIZE: u64 = 4096;
pub const MAX_USABLE_REGIONS: usize = 64;
const MAX_RECYCLED_FRAMES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FrameRegion {
    start: u64,
    end: u64,
}

impl FrameRegion {
    const fn empty() -> Self {
        Self { start: 0, end: 0 }
    }
}

pub struct FrameAllocator {
    regions: [FrameRegion; MAX_USABLE_REGIONS],
    region_count: usize,
    current_region: usize,
    next_frame: u64,
    usable_bytes: u64,
    allocated_frames: u64,
    recycled: [u64; MAX_RECYCLED_FRAMES],
    recycled_count: usize,
}

impl FrameAllocator {
    pub const fn new() -> Self {
        Self {
            regions: [FrameRegion::empty(); MAX_USABLE_REGIONS],
            region_count: 0,
            current_region: 0,
            next_frame: 0,
            usable_bytes: 0,
            allocated_frames: 0,
            recycled: [0; MAX_RECYCLED_FRAMES],
            recycled_count: 0,
        }
    }

    pub fn add_region(&mut self, region: MemoryRegion) {
        if region.kind != MemoryRegionKind::Usable
            || self.region_count >= MAX_USABLE_REGIONS
            || self.allocated_frames != 0
        {
            return;
        }

        // Keep the null page permanently unmapped/unallocated so accidental null pointers
        // cannot become valid physical-memory writes.
        let start = align_up(region.start.max(PAGE_SIZE), PAGE_SIZE);
        let raw_end = region.start.saturating_add(region.size);
        let end = align_down(raw_end, PAGE_SIZE);
        if start >= end {
            return;
        }

        let mut insert_at = self.region_count;
        while insert_at > 0 && self.regions[insert_at - 1].start > start {
            self.regions[insert_at] = self.regions[insert_at - 1];
            insert_at -= 1;
        }
        self.regions[insert_at] = FrameRegion { start, end };
        self.region_count += 1;
        self.usable_bytes = self.usable_bytes.saturating_add(end - start);
        self.next_frame = self.regions[0].start;
    }

    pub fn alloc_frame(&mut self) -> Option<u64> {
        if self.recycled_count > 0 {
            self.recycled_count -= 1;
            let frame = self.recycled[self.recycled_count];
            self.recycled[self.recycled_count] = 0;
            self.allocated_frames += 1;
            return Some(frame);
        }
        while self.current_region < self.region_count {
            let region = self.regions[self.current_region];
            if self.next_frame < region.start {
                self.next_frame = region.start;
            }
            if self.next_frame.saturating_add(PAGE_SIZE) <= region.end {
                let frame = self.next_frame;
                self.next_frame += PAGE_SIZE;
                self.allocated_frames += 1;
                return Some(frame);
            }
            self.current_region += 1;
            if self.current_region < self.region_count {
                self.next_frame = self.regions[self.current_region].start;
            }
        }
        None
    }

    pub fn free_frame(&mut self, frame: u64) -> bool {
        if frame == 0
            || !frame.is_multiple_of(PAGE_SIZE)
            || self.recycled_count >= MAX_RECYCLED_FRAMES
            || self.allocated_frames == 0
            || !self.was_allocated(frame)
            || self.recycled[..self.recycled_count].contains(&frame)
        {
            return false;
        }
        self.recycled[self.recycled_count] = frame;
        self.recycled_count += 1;
        self.allocated_frames -= 1;
        true
    }

    pub const fn usable_bytes(&self) -> u64 {
        self.usable_bytes
    }

    pub const fn allocated_frames(&self) -> u64 {
        self.allocated_frames
    }

    pub const fn region_count(&self) -> usize {
        self.region_count
    }

    pub const fn recycled_frames(&self) -> usize {
        self.recycled_count
    }

    fn was_allocated(&self, frame: u64) -> bool {
        self.regions
            .iter()
            .take(self.region_count)
            .enumerate()
            .any(|(index, region)| {
                frame >= region.start
                    && frame < region.end
                    && (index < self.current_region
                        || (index == self.current_region && frame < self.next_frame))
            })
    }
}

impl Default for FrameAllocator {
    fn default() -> Self {
        Self::new()
    }
}

const fn align_up(value: u64, align: u64) -> u64 {
    value.saturating_add(align - 1) & !(align - 1)
}

const fn align_down(value: u64, align: u64) -> u64 {
    value & !(align - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(start: u64, size: u64, kind: MemoryRegionKind) -> MemoryRegion {
        MemoryRegion { start, size, kind }
    }

    #[test]
    fn allocator_never_crosses_reserved_gaps() {
        let mut allocator = FrameAllocator::new();
        allocator.add_region(region(0x1003, 0x2ffd, MemoryRegionKind::Usable));
        allocator.add_region(region(0x9000, 0x2000, MemoryRegionKind::Usable));

        assert_eq!(allocator.alloc_frame(), Some(0x2000));
        assert_eq!(allocator.alloc_frame(), Some(0x3000));
        assert_eq!(allocator.alloc_frame(), Some(0x9000));
        assert_eq!(allocator.alloc_frame(), Some(0xa000));
        assert_eq!(allocator.alloc_frame(), None);
    }

    #[test]
    fn allocator_ignores_non_usable_and_partial_pages() {
        let mut allocator = FrameAllocator::new();
        allocator.add_region(region(0x1000, 0x4000, MemoryRegionKind::Reserved));
        allocator.add_region(region(0x7001, 0xffe, MemoryRegionKind::Usable));
        allocator.add_region(region(0x8001, 0x1fff, MemoryRegionKind::Usable));

        assert_eq!(allocator.region_count(), 1);
        assert_eq!(allocator.usable_bytes(), PAGE_SIZE);
        assert_eq!(allocator.alloc_frame(), Some(0x9000));
        assert_eq!(allocator.allocated_frames(), 1);
    }

    #[test]
    fn allocator_reserves_the_null_page() {
        let mut allocator = FrameAllocator::new();
        allocator.add_region(region(0, 0x3000, MemoryRegionKind::Usable));

        assert_eq!(allocator.alloc_frame(), Some(0x1000));
        assert_eq!(allocator.alloc_frame(), Some(0x2000));
        assert_eq!(allocator.alloc_frame(), None);
    }

    #[test]
    fn allocator_orders_firmware_regions_before_use() {
        let mut allocator = FrameAllocator::new();
        allocator.add_region(region(0x9000, 0x1000, MemoryRegionKind::Usable));
        allocator.add_region(region(0x3000, 0x1000, MemoryRegionKind::Usable));

        assert_eq!(allocator.alloc_frame(), Some(0x3000));
        assert_eq!(allocator.alloc_frame(), Some(0x9000));
    }

    #[test]
    fn allocator_layout_is_immutable_after_first_allocation() {
        let mut allocator = FrameAllocator::new();
        allocator.add_region(region(0x3000, 0x1000, MemoryRegionKind::Usable));
        assert_eq!(allocator.alloc_frame(), Some(0x3000));

        allocator.add_region(region(0x1000, 0x1000, MemoryRegionKind::Usable));
        assert_eq!(allocator.alloc_frame(), None);
    }

    #[test]
    fn freed_frames_are_reused_without_double_free() {
        let mut allocator = FrameAllocator::new();
        allocator.add_region(region(0x1000, 0x4000, MemoryRegionKind::Usable));
        let first = allocator.alloc_frame().unwrap();
        let second = allocator.alloc_frame().unwrap();

        assert!(allocator.free_frame(first));
        assert!(!allocator.free_frame(first));
        assert!(!allocator.free_frame(0x4000));
        assert_eq!(allocator.recycled_frames(), 1);
        assert_eq!(allocator.allocated_frames(), 1);
        assert_eq!(allocator.alloc_frame(), Some(first));
        assert_eq!(allocator.allocated_frames(), 2);
        assert_eq!(second, 0x2000);
    }
}
