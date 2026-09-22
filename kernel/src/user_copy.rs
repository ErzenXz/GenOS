//! Bounded, fully planned user copies. A plan contains addresses, never borrowed
//! user memory. The architecture adapter validates every page before any write
//! and keeps the address space stable until the complete copy finishes.
pub const PAGE_SIZE: u64 = 4096;
pub const MAX_COPY: usize = 4096;
pub const USER_BASE: u64 = 0x0000_4000_0000_0000;
pub const USER_END: u64 = USER_BASE + (1 << 39);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Chunk {
    pub page: u64,
    pub offset: usize,
    pub length: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Plan {
    chunks: [Chunk; 2],
    count: usize,
}

impl Plan {
    pub fn new(address: u64, length: usize) -> Option<Self> {
        let end = address.checked_add(length as u64)?;
        if length > MAX_COPY || !(USER_BASE..USER_END).contains(&address) || end > USER_END {
            return None;
        }
        let mut plan = Self {
            chunks: [Chunk::default(); 2],
            count: 0,
        };
        let mut cursor = address;
        while cursor < end {
            let offset = (cursor % PAGE_SIZE) as usize;
            let length = (end - cursor).min(PAGE_SIZE - offset as u64) as usize;
            plan.chunks[plan.count] = Chunk {
                page: cursor - offset as u64,
                offset,
                length,
            };
            plan.count += 1;
            cursor += length as u64;
        }
        Some(plan)
    }

    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks[..self.count]
    }

    /// Resolve the whole copy before transferring bytes. A missing/forbidden
    /// later page must not permit a partial copy into the first page.
    pub fn resolve(&self, mut page: impl FnMut(u64) -> Option<u64>) -> Option<[u64; 2]> {
        let mut physical = [0; 2];
        for (index, chunk) in self.chunks().iter().enumerate() {
            let base = page(chunk.page)?;
            if base == 0 || !base.is_multiple_of(PAGE_SIZE) {
                return None;
            }
            physical[index] = base.checked_add(chunk.offset as u64)?;
        }
        Some(physical)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_alignment_and_length_covers_exactly_the_original_range() {
        for offset in 0..PAGE_SIZE {
            for length in [0, 1, 80, 512, MAX_COPY] {
                let start = USER_BASE + offset;
                let plan = Plan::new(start, length).unwrap();
                let mut cursor = start;
                for chunk in plan.chunks() {
                    assert!(chunk.page.is_multiple_of(PAGE_SIZE));
                    assert_eq!(chunk.page + chunk.offset as u64, cursor);
                    assert!(chunk.length > 0 && chunk.offset + chunk.length <= PAGE_SIZE as usize);
                    cursor += chunk.length as u64;
                }
                assert_eq!(cursor, start + length as u64);
            }
        }
    }

    #[test]
    fn overflowing_noncanonical_kernel_and_oversized_ranges_fail() {
        for (address, length) in [
            (u64::MAX, 2),
            (USER_BASE | (1 << 63), 1),
            (USER_BASE - 1, 1),
            (USER_END, 0),
            (USER_END - 1, 2),
            (USER_BASE, MAX_COPY + 1),
        ] {
            assert_eq!(Plan::new(address, length), None);
        }
        assert!(Plan::new(USER_END - 1, 1).is_some());
    }

    #[test]
    fn later_page_failure_yields_no_transfer_plan() {
        let plan = Plan::new(USER_BASE + PAGE_SIZE - 1, 2).unwrap();
        assert_eq!(
            plan.resolve(|page| (page == USER_BASE).then_some(0x1000)),
            None
        );
        assert_eq!(plan.resolve(|_| Some(0)), None);
        assert_eq!(plan.resolve(|_| Some(0x1001)), None);
        assert_eq!(
            plan.resolve(|page| Some(page - USER_BASE + 0x1000)),
            Some([0x1fff, 0x2000])
        );
    }
}
