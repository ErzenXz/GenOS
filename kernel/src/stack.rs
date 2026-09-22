//! Storage geometry for statically reserved, downward-growing kernel stacks.

pub const GUARD_BYTES: usize = 4096;
pub const WATERMARK_WORD: u64 = 0xa5a5_a5a5_a5a5_a5a5;
pub const MIN_REMAINING_BYTES: usize = GUARD_BYTES;

/// Return the deepest overwritten word's distance from the top. A byte pattern
/// measures touched storage, with eight-byte granularity; it is not a sampled
/// RSP or a promise that arbitrary data cannot equal the initialization pattern.
pub fn touched_bytes(words: usize, mut read: impl FnMut(usize) -> u64) -> Option<usize> {
    let capacity = words.checked_mul(core::mem::size_of::<u64>())?;
    for index in 0..words {
        if read(index) != WATERMARK_WORD {
            return Some(capacity - index * core::mem::size_of::<u64>());
        }
    }
    Some(0)
}

/// Guard storage remains part of the reserved kernel image, but its identity
/// mappings are removed before interrupts or processes can use these stacks.
#[repr(C, align(4096))]
pub struct GuardedStack<const N: usize> {
    _lower_guard: [u8; GUARD_BYTES],
    _usable: [u8; N],
    _upper_guard: [u8; GUARD_BYTES],
}

impl<const N: usize> GuardedStack<N> {
    pub const fn new() -> Self {
        assert!(N != 0 && N.is_multiple_of(GUARD_BYTES));
        Self {
            _lower_guard: [0; GUARD_BYTES],
            _usable: [0; N],
            _upper_guard: [0; GUARD_BYTES],
        }
    }
}

impl<const N: usize> Default for GuardedStack<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Layout {
    pub lower_guard: u64,
    pub usable_start: u64,
    /// Empty stack pointer; the first push writes below this address.
    pub top: u64,
    pub allocation_end: u64,
}

impl Layout {
    pub fn new(base: u64, usable_bytes: usize) -> Option<Self> {
        if base == 0
            || !base.is_multiple_of(GUARD_BYTES as u64)
            || usable_bytes == 0
            || !usable_bytes.is_multiple_of(GUARD_BYTES)
        {
            return None;
        }
        let usable_start = base.checked_add(GUARD_BYTES as u64)?;
        let top = usable_start.checked_add(u64::try_from(usable_bytes).ok()?)?;
        let allocation_end = top.checked_add(GUARD_BYTES as u64)?;
        Some(Self {
            lower_guard: base,
            usable_start,
            top,
            allocation_end,
        })
    }

    pub const fn guards(self) -> [u64; 2] {
        [self.lower_guard, self.top]
    }

    pub const fn contains_stack_pointer(self, pointer: u64) -> bool {
        pointer >= self.usable_start && pointer <= self.top
    }

    pub const fn overlaps(self, other: Self) -> bool {
        self.lower_guard < other.allocation_end && other.lower_guard < self.allocation_end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_has_dedicated_page_guards_and_exact_usable_capacity() {
        fn check<const N: usize>() {
            assert_eq!(core::mem::align_of::<GuardedStack<N>>(), GUARD_BYTES);
            assert_eq!(core::mem::offset_of!(GuardedStack<N>, _usable), GUARD_BYTES);
            assert_eq!(
                core::mem::offset_of!(GuardedStack<N>, _upper_guard),
                GUARD_BYTES + N
            );
            assert_eq!(core::mem::size_of::<GuardedStack<N>>(), N + 2 * GUARD_BYTES);
        }
        check::<{ 2 * 1024 * 1024 }>();
        check::<{ 64 * 1024 }>();
        check::<{ 16 * 1024 }>();
    }

    #[test]
    fn pointer_boundary_and_guard_geometry_are_distinct() {
        let layout = Layout::new(0x20_0000, 64 * 1024).unwrap();
        assert_eq!(layout.guards(), [0x20_0000, 0x21_1000]);
        assert_eq!(layout.usable_start, 0x20_1000);
        assert_eq!(layout.allocation_end, 0x21_2000);
        assert!(layout.contains_stack_pointer(layout.top));
        assert!(layout.contains_stack_pointer(layout.usable_start));
        assert!(!layout.contains_stack_pointer(layout.usable_start - 1));
        assert!(!layout.contains_stack_pointer(layout.top + 1));
    }

    #[test]
    fn malformed_or_overflowing_geometry_is_rejected() {
        for (base, size) in [
            (0, 4096),
            (1, 4096),
            (4096, 0),
            (4096, 4095),
            (u64::MAX - 4095, 4096),
            (4096, usize::MAX),
        ] {
            assert!(Layout::new(base, size).is_none());
        }
    }

    #[test]
    fn adjoining_allocations_do_not_share_a_guard_or_usable_page() {
        let first = Layout::new(0x20_0000, 4096).unwrap();
        let second = Layout::new(first.allocation_end, 4096).unwrap();
        assert!(!first.overlaps(second));
        assert!(!second.overlaps(first));
        assert!(first.overlaps(first));
        assert!(first.overlaps(Layout::new(first.top, 4096).unwrap()));
        assert_ne!(first.guards()[1], second.guards()[0]);
    }

    #[test]
    fn watermark_measures_deepest_touch_and_never_reads_outside_capacity() {
        let mut words = [WATERMARK_WORD; 64];
        assert_eq!(touched_bytes(words.len(), |index| words[index]), Some(0));
        words[63] = 0;
        assert_eq!(touched_bytes(words.len(), |index| words[index]), Some(8));
        words[16] = 0;
        assert_eq!(touched_bytes(words.len(), |index| words[index]), Some(384));
        words[0] = 0;
        assert_eq!(touched_bytes(words.len(), |index| words[index]), Some(512));
        assert_eq!(touched_bytes(0, |_| panic!("empty scan read")), Some(0));
        assert_eq!(
            touched_bytes(usize::MAX, |_| panic!("overflow scan read")),
            None
        );
    }
}
