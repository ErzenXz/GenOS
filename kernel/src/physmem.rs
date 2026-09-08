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

/// A by-value diagnostic snapshot. Operation counters saturate rather than wrap.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllocatorStats {
    pub total: u64,
    pub live: u64,
    pub free: u64,
    pub peak_live: u64,
    pub allocations: u64,
    pub releases: u64,
    pub allocation_failures: u64,
    pub invalid_releases: u64,
    pub preparation_failures: u64,
    pub regions: usize,
}

impl AllocatorStats {
    pub fn write_report(
        &self,
        out: &mut impl core::fmt::Write,
        consistent: bool,
    ) -> core::fmt::Result {
        writeln!(out, "memory allocator")?;
        writeln!(
            out,
            "frames_total={} live={} free={}",
            self.total, self.live, self.free
        )?;
        writeln!(out, "peak_live={} regions={}", self.peak_live, self.regions)?;
        writeln!(
            out,
            "allocations={} releases={}",
            self.allocations, self.releases
        )?;
        writeln!(out, "allocation_failures={}", self.allocation_failures)?;
        writeln!(out, "invalid_releases={}", self.invalid_releases)?;
        writeln!(out, "preparation_failures={}", self.preparation_failures)?;
        writeln!(out, "consistent={}", if consistent { "yes" } else { "NO" })
    }
}

/// Erase exactly one owned page with stores the compiler cannot discard.
/// This does not promise removal from caches, swap, devices, or physical remanence.
pub fn scrub_page(page: &mut [u8; PAGE_SIZE as usize]) {
    for byte in page {
        // SAFETY: each pointer refers to an initialized byte in this exclusively
        // borrowed array. Byte stores need no stronger alignment and cannot escape it.
        unsafe {
            core::ptr::write_volatile(byte, 0);
        }
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
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
    peak_live: u64,
    allocations: u64,
    releases: u64,
    allocation_failures: u64,
    invalid_releases: u64,
    preparation_failures: u64,
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
            peak_live: 0,
            allocations: 0,
            releases: 0,
            allocation_failures: 0,
            invalid_releases: 0,
            preparation_failures: 0,
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
            self.peak_live = self.peak_live.max(self.allocated_frames);
            self.allocations = self.allocations.saturating_add(1);
            self.started = true;
            return Some(frame);
        }
        self.allocation_failures = self.allocation_failures.saturating_add(1);
        None
    }

    /// Metadata-only release for host models. Physical adapters must prepare
    /// memory (for example scrub it) through `free_frame_with` before reuse.
    pub fn free_frame(&mut self, frame: u64) -> bool {
        self.free_frame_with(frame, |_| true)
    }

    /// Validate first, prepare while still owned, then publish the free bit.
    /// A denied address never reaches `prepare`; a failed preparation retains
    /// the grant. This validates a live grant, not a caller's ownership token.
    pub fn free_frame_with(&mut self, frame: u64, prepare: impl FnOnce(u64) -> bool) -> bool {
        let Some(index) = self.live_index(frame) else {
            self.invalid_releases = self.invalid_releases.saturating_add(1);
            return false;
        };
        if !prepare(frame) {
            self.preparation_failures = self.preparation_failures.saturating_add(1);
            return false;
        }
        let word = index / 64;
        self.allocated[word] &= !(1u64 << (index % 64));
        self.allocated_frames -= 1;
        self.releases = self.releases.saturating_add(1);
        self.next_word = self.next_word.min(word);
        true
    }

    fn live_index(&self, frame: u64) -> Option<usize> {
        if frame == 0 || !frame.is_multiple_of(PAGE_SIZE) {
            return None;
        }
        let region = self.regions[..self.region_count]
            .iter()
            .find(|region| frame >= region.start && frame < region.end)?;
        let index = region.first + ((frame - region.start) / PAGE_SIZE) as usize;
        (self.allocated[index / 64] & (1u64 << (index % 64)) != 0).then_some(index)
    }

    pub fn stats(&self) -> AllocatorStats {
        AllocatorStats {
            total: self.frame_count as u64,
            live: self.allocated_frames,
            free: (self.frame_count as u64).saturating_sub(self.allocated_frames),
            peak_live: self.peak_live,
            allocations: self.allocations,
            releases: self.releases,
            allocation_failures: self.allocation_failures,
            invalid_releases: self.invalid_releases,
            preparation_failures: self.preparation_failures,
            regions: self.region_count,
        }
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
        if self.region_count > MAX_USABLE_REGIONS
            || self.frame_count > WORDS.saturating_mul(64)
            || self.high_water > self.frame_count
            || self.allocated_frames > self.high_water as u64
            || self.peak_live < self.allocated_frames
            || self.peak_live > self.frame_count as u64
            || (self.allocated_frames != 0 && !self.started)
        {
            return false;
        }
        let used = self.frame_count.div_ceil(64);
        if self.next_word > used || self.allocated[used..].iter().any(|word| *word != 0) {
            return false;
        }
        let tail = self.frame_count % 64;
        if tail != 0 && self.allocated[used - 1] >> tail != 0 {
            return false;
        }
        let mut first = 0usize;
        let mut end = PAGE_SIZE;
        for region in &self.regions[..self.region_count] {
            if region.start < end
                || region.start >= region.end
                || region.first != first
                || !region.start.is_multiple_of(PAGE_SIZE)
                || !region.end.is_multiple_of(PAGE_SIZE)
            {
                return false;
            }
            first += region.frames();
            end = region.end;
        }
        first == self.frame_count
            && self.allocated[..used]
                .iter()
                .map(|word| u64::from(word.count_ones()))
                .sum::<u64>()
                == self.allocated_frames
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
    fn release_preparation_runs_only_for_a_live_grant_and_failure_retains_it() {
        let mut allocator = FrameAllocator::<1>::new();
        allocator.add_region(region(0x1000, PAGE_SIZE, MemoryRegionKind::Usable));
        for frame in [0, 0x1000, 0x1001, 0x2000] {
            assert!(!allocator.free_frame_with(frame, |_| panic!("invalid address reached memory")));
        }
        let frame = allocator.alloc_frame().unwrap();
        assert!(!allocator.free_frame_with(frame, |actual| {
            assert_eq!(actual, frame);
            false
        }));
        assert_eq!(allocator.stats().live, 1);
        assert_eq!(allocator.stats().preparation_failures, 1);
        assert!(allocator.alloc_frame().is_none());
        assert!(allocator.free_frame_with(frame, |_| true));
        assert!(!allocator.free_frame_with(frame, |_| panic!("double free reached memory")));
        assert_eq!(allocator.alloc_frame(), Some(frame));
        assert!(allocator.is_consistent());
    }

    #[test]
    fn scrubbing_erases_every_byte_and_preserves_adjacent_storage() {
        struct Storage {
            before: [u8; 17],
            page: [u8; 4096],
            after: [u8; 19],
        }
        let mut storage = Storage {
            before: [0x17; 17],
            page: [0xa5; 4096],
            after: [0x19; 19],
        };
        scrub_page(&mut storage.page);
        assert!(storage.page.iter().all(|byte| *byte == 0));
        assert_eq!(storage.before, [0x17; 17]);
        assert_eq!(storage.after, [0x19; 19]);
    }

    #[test]
    fn statistics_distinguish_live_peak_reuse_exhaustion_and_invalid_release() {
        let mut allocator = FrameAllocator::<1>::new();
        allocator.add_region(region(0x1000, PAGE_SIZE * 2, MemoryRegionKind::Usable));
        let first = allocator.alloc_frame().unwrap();
        let second = allocator.alloc_frame().unwrap();
        assert!(allocator.alloc_frame().is_none());
        assert!(allocator.free_frame(first));
        assert!(!allocator.free_frame(first));
        assert_eq!(allocator.alloc_frame(), Some(first));
        assert!(allocator.free_frame(second));
        let stats = allocator.stats();
        assert_eq!(
            (stats.total, stats.live, stats.free, stats.peak_live),
            (2, 1, 1, 2)
        );
        assert_eq!(
            (
                stats.allocations,
                stats.releases,
                stats.allocation_failures,
                stats.invalid_releases
            ),
            (3, 2, 1, 1)
        );
        let mut report = std::string::String::new();
        stats
            .write_report(&mut report, allocator.is_consistent())
            .unwrap();
        assert!(report.contains("consistent=yes"));
        assert!(report.contains("allocations=3 releases=2"));
        assert!(report.len() < 512);
    }

    #[test]
    fn consistency_checks_region_geometry_and_bits_outside_managed_memory() {
        let mut allocator = FrameAllocator::<2>::new();
        allocator.add_region(region(0x1000, PAGE_SIZE, MemoryRegionKind::Usable));
        assert!(allocator.is_consistent());
        // A matching population count alone must not bless a bit outside the map.
        allocator.allocated[1] = 1;
        allocator.allocated_frames = 1;
        allocator.high_water = 1;
        allocator.peak_live = 1;
        allocator.started = true;
        assert!(!allocator.is_consistent());
        allocator.allocated[1] = 0;
        allocator.allocated[0] = 1;
        assert!(allocator.is_consistent());
        allocator.regions[0].first = 1;
        assert!(!allocator.is_consistent());
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
