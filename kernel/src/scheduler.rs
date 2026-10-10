//! Bounded round-robin policy without process, clock or hardware authority.
//!
//! A scan snapshots the last selected slot, visits each slot once, and starts
//! after it. Only completed selection advances the cursor; skipped and idle
//! scans preserve it. The coordinator owns readiness and dispatch/cleanup.

#[derive(Clone, Copy, Debug)]
pub struct RoundRobin<const N: usize> {
    last: usize,
}

impl<const N: usize> Default for RoundRobin<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> RoundRobin<N> {
    pub const fn new() -> Self {
        Self { last: 0 }
    }

    pub const fn scan(&self) -> Scan<N> {
        Scan {
            last: self.last,
            remaining: N,
        }
    }

    /// A forged/out-of-capacity selection must not alter future ordering.
    pub fn selected(&mut self, index: usize) -> bool {
        if index >= N {
            return false;
        }
        self.last = index;
        true
    }
}

pub struct Scan<const N: usize> {
    last: usize,
    remaining: usize,
}

impl<const N: usize> Iterator for Scan<N> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        if self.remaining == 0 {
            return None;
        }
        // Private cursor is in range for nonempty scans. Avoid addition and
        // modulo overflow even at the largest representable slot capacity.
        self.last = if self.last == N - 1 { 0 } else { self.last + 1 };
        self.remaining -= 1;
        Some(self.last)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<const N: usize> ExactSizeIterator for Scan<N> {}
impl<const N: usize> core::iter::FusedIterator for Scan<N> {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    #[test]
    fn each_scan_is_bounded_unique_and_wraps_after_the_last_selection() {
        let mut schedule = RoundRobin::<4>::new();
        assert_eq!(schedule.scan().collect::<Vec<_>>(), [1, 2, 3, 0]);
        for last in 0..4 {
            assert!(schedule.selected(last));
            let order: Vec<_> = schedule.scan().collect();
            assert_eq!(order.len(), 4);
            for (offset, index) in order.into_iter().enumerate() {
                assert_eq!(index, (last + offset + 1) % 4);
            }
        }
    }

    #[test]
    fn every_ready_subset_gets_bounded_service_without_visiting_blocked_slots() {
        for mask in 1u8..16 {
            let mut schedule = RoundRobin::<4>::new();
            let mut counts = [0; 4];
            for _ in 0..mask.count_ones() * 16 {
                let index = schedule.scan().find(|&i| mask & (1 << i) != 0).unwrap();
                assert!(schedule.selected(index));
                counts[index] += 1;
            }
            for (index, count) in counts.into_iter().enumerate() {
                assert_eq!(count, if mask & (1 << index) != 0 { 16 } else { 0 });
            }
        }
    }

    #[test]
    fn idle_invalid_and_partially_consumed_scans_preserve_cursor() {
        let mut schedule = RoundRobin::<4>::new();
        assert!(schedule.selected(2));
        let before: Vec<_> = schedule.scan().collect();
        assert_eq!(schedule.scan().next(), Some(3));
        assert_eq!(schedule.scan().find(|_| false), None);
        for index in [4, 5, usize::MAX] {
            assert!(!schedule.selected(index));
            assert_eq!(schedule.scan().collect::<Vec<_>>(), before);
        }
    }

    #[test]
    fn bounded_wakeup_batches_keep_snapshot_order_and_rotate_between_owners() {
        let mut schedule = RoundRobin::<4>::new();
        let mut counts = [0; 4];
        for _ in 0..16 {
            let scan = schedule.scan();
            for index in scan.take(2) {
                counts[index] += 1;
                assert!(schedule.selected(index));
            }
        }
        assert_eq!(counts, [8; 4]);
    }

    #[test]
    fn empty_singleton_and_maximum_capacity_do_not_divide_or_overflow() {
        let mut empty = RoundRobin::<0>::new();
        assert_eq!(empty.scan().next(), None);
        assert!(!empty.selected(0));
        let mut one = RoundRobin::<1>::new();
        assert!(one.selected(0));
        let mut scan = one.scan();
        assert_eq!(scan.len(), 1);
        assert_eq!(scan.next(), Some(0));
        assert_eq!(scan.len(), 0);
        assert_eq!(scan.next(), None);
        assert_eq!(scan.next(), None);
        let mut largest = RoundRobin::<{ usize::MAX }>::new();
        assert!(largest.selected(usize::MAX - 1));
        assert_eq!(largest.scan().take(3).collect::<Vec<_>>(), [0, 1, 2]);
    }
}
