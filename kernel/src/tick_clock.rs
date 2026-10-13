//! Monotonic logical ticks across the first hardware timer delivery.
//!
//! Polling supplies progress only before hardware ticks begin. The caller must
//! serialize polling and IRQ updates; this object performs no synchronization.

pub const FALLBACK_POLL_PERIOD: usize = 4096;

pub struct Clock {
    ticks: u64,
    fallback_phase: usize,
    hardware_seen: bool,
}

impl Clock {
    pub const fn new() -> Self {
        Self {
            ticks: 0,
            fallback_phase: 0,
            hardware_seen: false,
        }
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    pub fn poll_fallback(&mut self) -> u64 {
        if !self.hardware_seen {
            self.fallback_phase += 1;
            if self.fallback_phase == FALLBACK_POLL_PERIOD {
                self.fallback_phase = 0;
                self.advance();
            }
        }
        self.ticks
    }

    pub fn hardware_tick(&mut self) -> u64 {
        self.hardware_seen = true;
        self.fallback_phase = 0;
        self.advance();
        self.ticks
    }

    fn advance(&mut self) {
        self.ticks = self.ticks.saturating_add(1);
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_disabled_polling_keeps_progress_and_a_bounded_phase() {
        let mut clock = Clock::new();
        for poll in 1..=FALLBACK_POLL_PERIOD * 4 + 17 {
            assert_eq!(clock.poll_fallback(), (poll / FALLBACK_POLL_PERIOD) as u64);
            assert_eq!(clock.fallback_phase, poll % FALLBACK_POLL_PERIOD);
        }
        assert!(!clock.hardware_seen);
    }

    #[test]
    fn first_hardware_tick_preserves_every_fallback_boundary() {
        for phase in 0..FALLBACK_POLL_PERIOD {
            let mut clock = Clock {
                ticks: 3,
                fallback_phase: phase,
                hardware_seen: false,
            };
            assert_eq!(clock.ticks(), 3);
            assert_eq!(clock.hardware_tick(), 4);
            assert_eq!(clock.fallback_phase, 0);
            for _ in 0..2 {
                assert_eq!(clock.poll_fallback(), 4);
            }
            assert_eq!(clock.hardware_tick(), 5);
        }
    }

    #[test]
    fn polling_and_hardware_interleavings_never_reverse_or_double_count() {
        for first_irq in [
            0,
            1,
            FALLBACK_POLL_PERIOD - 1,
            FALLBACK_POLL_PERIOD,
            FALLBACK_POLL_PERIOD + 1,
            FALLBACK_POLL_PERIOD * 2 - 1,
            FALLBACK_POLL_PERIOD * 2,
            FALLBACK_POLL_PERIOD * 2 + 1,
        ] {
            let mut clock = Clock::new();
            for _ in 0..first_irq {
                clock.poll_fallback();
            }
            let fallback_ticks = (first_irq / FALLBACK_POLL_PERIOD) as u64;
            assert_eq!(clock.hardware_tick(), fallback_ticks + 1);
            for irq in 2..=8 {
                for _ in 0..17 {
                    assert_eq!(clock.poll_fallback(), fallback_ticks + irq - 1);
                }
                assert_eq!(clock.hardware_tick(), fallback_ticks + irq);
            }
        }
    }

    #[test]
    fn hardware_from_boot_stops_fallback_across_multiple_poll_periods() {
        let mut clock = Clock::new();
        assert_eq!(clock.hardware_tick(), 1);
        for _ in 0..FALLBACK_POLL_PERIOD * 2 + 1 {
            assert_eq!(clock.poll_fallback(), 1);
            assert_eq!(clock.fallback_phase, 0);
        }
        assert_eq!(clock.hardware_tick(), 2);
    }

    #[test]
    fn logical_ticks_saturate_through_polling_and_hardware_transition() {
        let mut clock = Clock {
            ticks: u64::MAX - 1,
            fallback_phase: FALLBACK_POLL_PERIOD - 1,
            hardware_seen: false,
        };
        assert_eq!(clock.poll_fallback(), u64::MAX);
        for _ in 0..FALLBACK_POLL_PERIOD {
            assert_eq!(clock.poll_fallback(), u64::MAX);
        }
        assert_eq!(clock.hardware_tick(), u64::MAX);
        assert_eq!(clock.hardware_tick(), u64::MAX);
        assert_eq!(clock.poll_fallback(), u64::MAX);
        assert_eq!(clock.fallback_phase, 0);

        let mut hardware_clock = Clock {
            ticks: u64::MAX - 1,
            fallback_phase: 0,
            hardware_seen: true,
        };
        assert_eq!(hardware_clock.hardware_tick(), u64::MAX);
        assert_eq!(hardware_clock.hardware_tick(), u64::MAX);
    }
}
