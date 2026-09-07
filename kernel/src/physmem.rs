use genos_abi::{MemoryRegion, MemoryRegionKind};

pub const PAGE_SIZE: u64 = 4096;
pub const MAX_USABLE_REGIONS: usize = 64;
// One bit per managed page. The kernel uses 32,768 words (8 GiB of usable
// frames); small host fixtures use the same implementation with fewer words.
pub const KERNEL_BITMAP_WORDS: usize = 32_768;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FrameRegion {
    start: u64,
    end: u64,
    first: usize,
}
impl FrameRegion {
    const fn empty() -> Self {
        Self {
            start: 0,
            end: 0,
            first: 0,
        }
    }
    fn frames(self) -> usize {
        ((self.end - self.start) / PAGE_SIZE) as usize
    }
}

pub struct FrameAllocator<const WORDS: usize = 64> {
    regions: [FrameRegion; MAX_USABLE_REGIONS],
    region_count: usize,
    allocated: [u64; WORDS],
    frame_count: usize,
    next_word: usize,
    high_water: usize,
    allocated_frames: u64,
    started: bool,
}

impl<const WORDS: usize> FrameAllocator<WORDS> {
    pub const fn new() -> Self {
        Self {
            regions: [FrameRegion::empty(); MAX_USABLE_REGIONS],
            region_count: 0,
            allocated: [0; WORDS],
            frame_count: 0,
            next_word: 0,
            high_water: 0,
            allocated_frames: 0,
            started: false,
        }
    }

    /// Freeze the layout after the first grant, even after all grants are
    /// returned. Malformed/overlapping maps and metadata exhaustion fail
    /// before mutation; the caller must not silently discard usable memory.
    pub fn add_region(&mut self, region: MemoryRegion) -> bool {
        if region.kind != MemoryRegionKind::Usable {
            return true;
        }
        if self.started {
            return false;
        }
        let start = align_up(region.start.max(PAGE_SIZE), PAGE_SIZE);
        let Some(raw_end) = region.start.checked_add(region.size) else {
            return false;
        };
        let end = align_down(raw_end, PAGE_SIZE);
        if start >= end {
            return true;
        }
        let Ok(frames) = usize::try_from((end - start) / PAGE_SIZE) else {
            return false;
        };
        let Some(total) = self.frame_count.checked_add(frames) else {
            return false;
        };
        if self.region_count == MAX_USABLE_REGIONS
            || total > WORDS.saturating_mul(64)
            || self.regions[..self.region_count]
                .iter()
                .any(|r| start < r.end && r.start < end)
        {
            return false;
        }
        let mut slot = self.region_count;
        while slot > 0 && self.regions[slot - 1].start > start {
            self.regions[slot] = self.regions[slot - 1];
            slot -= 1;
        }
        self.regions[slot] = FrameRegion {
            start,
            end,
            first: 0,
        };
        self.region_count += 1;
        let mut first = 0;
        for region in &mut self.regions[..self.region_count] {
            region.first = first;
            first += region.frames();
        }
        self.frame_count = total;
        self.next_word = 0;
        true
    }

    pub fn alloc_frame(&mut self) -> Option<u64> {
        while self.next_word < self.frame_count.div_ceil(64) {
            let base = self.next_word * 64;
            let remaining = (self.frame_count - base).min(64);
            let mask = if remaining == 64 {
                u64::MAX
            } else {
                (1u64 << remaining) - 1
            };
            let free = !self.allocated[self.next_word] & mask;
            if free == 0 {
                self.next_word += 1;
                continue;
            }
            let bit = free.trailing_zeros() as usize;
            let index = base + bit;
            let region = self.regions[..self.region_count]
                .iter()
                .find(|r| index >= r.first && index - r.first < r.frames())?;
            let frame = region.start + (index - region.first) as u64 * PAGE_SIZE;
            self.allocated[self.next_word] |= 1u64 << bit;
            self.high_water = self.high_water.max(index + 1);
            self.allocated_frames += 1;
            self.started = true;
            return Some(frame);
        }
        None
    }

    pub fn free_frame(&mut self, frame: u64) -> bool {
        if frame == 0 || !frame.is_multiple_of(PAGE_SIZE) {
            return false;
        }
        let Some(region) = self.regions[..self.region_count]
            .iter()
            .find(|r| frame >= r.start && frame < r.end)
        else {
            return false;
        };
        let index = region.first + ((frame - region.start) / PAGE_SIZE) as usize;
        let word = index / 64;
        let mask = 1u64 << (index % 64);
        if self.allocated[word] & mask == 0 {
            return false;
        }
        self.allocated[word] &= !mask;
        self.allocated_frames -= 1;
        self.next_word = self.next_word.min(word);
        true
    }

    pub const fn usable_bytes(&self) -> u64 {
        self.frame_count as u64 * PAGE_SIZE
    }
    pub const fn allocated_frames(&self) -> u64 {
        self.allocated_frames
    }
    pub const fn region_count(&self) -> usize {
        self.region_count
    }
    pub const fn recycled_frames(&self) -> usize {
        self.high_water - self.allocated_frames as usize
    }
    pub fn is_consistent(&self) -> bool {
        self.allocated
            .iter()
            .map(|word| u64::from(word.count_ones()))
            .sum::<u64>()
            == self.allocated_frames
            && self.high_water <= self.frame_count
            && self.allocated_frames <= self.high_water as u64
    }
}

impl<const WORDS: usize> Default for FrameAllocator<WORDS> {
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
        let mut allocator = FrameAllocator::<64>::new();
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
        let mut allocator = FrameAllocator::<64>::new();
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
        let mut allocator = FrameAllocator::<64>::new();
        allocator.add_region(region(0, 0x3000, MemoryRegionKind::Usable));

        assert_eq!(allocator.alloc_frame(), Some(0x1000));
        assert_eq!(allocator.alloc_frame(), Some(0x2000));
        assert_eq!(allocator.alloc_frame(), None);
    }

    #[test]
    fn allocator_orders_firmware_regions_before_use() {
        let mut allocator = FrameAllocator::<64>::new();
        allocator.add_region(region(0x9000, 0x1000, MemoryRegionKind::Usable));
        allocator.add_region(region(0x3000, 0x1000, MemoryRegionKind::Usable));

        assert_eq!(allocator.alloc_frame(), Some(0x3000));
        assert_eq!(allocator.alloc_frame(), Some(0x9000));
    }

    #[test]
    fn allocator_layout_is_immutable_after_first_allocation() {
        let mut allocator = FrameAllocator::<64>::new();
        allocator.add_region(region(0x3000, 0x1000, MemoryRegionKind::Usable));
        assert_eq!(allocator.alloc_frame(), Some(0x3000));

        allocator.add_region(region(0x1000, 0x1000, MemoryRegionKind::Usable));
        assert_eq!(allocator.alloc_frame(), None);
    }

    #[test]
    fn freed_frames_are_reused_without_double_free() {
        let mut allocator = FrameAllocator::<64>::new();
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

    #[test]
    fn more_than_256_reclaimed_pages_remain_lossless() {
        let mut allocator = FrameAllocator::<32>::new();
        assert!(allocator.add_region(region(0x1000, PAGE_SIZE * 1025, MemoryRegionKind::Usable)));
        let frames: std::vec::Vec<_> = (0..1025)
            .map(|_| allocator.alloc_frame().unwrap())
            .collect();
        assert!(allocator.alloc_frame().is_none());
        for &frame in frames.iter().rev() {
            assert!(allocator.free_frame(frame));
        }
        assert_eq!(allocator.recycled_frames(), 1025);
        assert!(allocator.is_consistent());
        for &frame in &frames {
            assert_eq!(allocator.alloc_frame(), Some(frame));
        }
        assert!(allocator.alloc_frame().is_none());
        assert!(allocator.is_consistent());
    }

    #[test]
    fn overlap_exhaustion_and_mutation_after_all_frames_return_fail_closed() {
        let mut allocator = FrameAllocator::<1>::new();
        assert!(!allocator.add_region(region(u64::MAX - 8191, 16384, MemoryRegionKind::Usable)));
        assert_eq!(allocator.region_count(), 0);
        assert!(allocator.add_region(region(0x2000, PAGE_SIZE * 32, MemoryRegionKind::Usable)));
        assert!(!allocator.add_region(region(0x1000, PAGE_SIZE * 2, MemoryRegionKind::Usable)));
        assert!(!allocator.add_region(region(0x100000, PAGE_SIZE * 33, MemoryRegionKind::Usable)));
        assert_eq!(allocator.region_count(), 1);
        let frame = allocator.alloc_frame().unwrap();
        assert!(allocator.free_frame(frame));
        assert!(!allocator.add_region(region(0x900000, PAGE_SIZE, MemoryRegionKind::Usable)));
        assert_eq!(allocator.alloc_frame(), Some(frame));
    }

    #[test]
    fn randomized_fragmented_allocations_never_alias_live_grants() {
        use std::collections::BTreeSet;
        let mut allocator = FrameAllocator::<16>::new();
        for index in 0..8 {
            assert!(allocator.add_region(region(
                0x1000 + index * 0x100000,
                PAGE_SIZE * 100,
                MemoryRegionKind::Usable
            )));
        }
        let mut live = BTreeSet::new();
        let mut random = 0x1234_5678u64;
        for _ in 0..20_000 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            if random & 3 == 0 && !live.is_empty() {
                let frame = *live.iter().nth((random as usize) % live.len()).unwrap();
                assert!(allocator.free_frame(frame));
                assert!(!allocator.free_frame(frame));
                live.remove(&frame);
            } else if let Some(frame) = allocator.alloc_frame() {
                assert!(live.insert(frame));
            }
            assert_eq!(allocator.allocated_frames(), live.len() as u64);
            assert!(allocator.is_consistent());
        }
        for frame in live {
            assert!(allocator.free_frame(frame));
        }
        assert_eq!(allocator.allocated_frames(), 0);
    }
}
