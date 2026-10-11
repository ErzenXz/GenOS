//! Saturating event totals. These counters carry no allocation or handle authority.
use core::sync::atomic::{AtomicU64, Ordering};

pub struct DiagnosticCounter(AtomicU64);

impl DiagnosticCounter {
    pub const fn new(initial: u64) -> Self {
        Self(AtomicU64::new(initial))
    }

    pub fn record(&self) -> u64 {
        self.add(1)
    }

    pub fn add(&self, amount: u64) -> u64 {
        // This closure always supplies an update. CAS retries preserve every
        // admitted event until saturation; neither atomic nor returned totals wrap.
        self.0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                Some(value.saturating_add(amount))
            })
            .unwrap()
            .saturating_add(amount)
    }

    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_boundary_does_not_panic_wrap_or_repeat_a_boot_milestone() {
        let counter = DiagnosticCounter::new(0);
        let mut milestones = 0;
        for expected in 1..=300 {
            let total = counter.record();
            assert_eq!(total, expected);
            milestones += usize::from(total == 3);
        }
        assert_eq!(counter.get(), 300);
        assert_eq!(milestones, 1);
    }

    #[test]
    fn totals_saturate_even_for_an_overflowing_increment() {
        let counter = DiagnosticCounter::new(u64::MAX - 1);
        assert_eq!(counter.add(0), u64::MAX - 1);
        assert_eq!(counter.add(2), u64::MAX);
        assert_eq!(counter.record(), u64::MAX);
        assert_eq!(counter.add(u64::MAX), u64::MAX);
        assert_eq!(counter.get(), u64::MAX);
    }

    #[test]
    fn concurrent_events_have_one_linearized_total_each() {
        let counter = std::sync::Arc::new(DiagnosticCounter::new(0));
        let handles: std::vec::Vec<_> = (0..4)
            .map(|_| {
                let counter = counter.clone();
                std::thread::spawn(move || {
                    (0..8)
                        .map(|_| counter.record())
                        .collect::<std::vec::Vec<_>>()
                })
            })
            .collect();
        let mut totals: std::vec::Vec<_> = handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect();
        totals.sort_unstable();
        assert_eq!(totals, (1..=32).collect::<std::vec::Vec<_>>());
        assert_eq!(counter.get(), 32);
    }
}
