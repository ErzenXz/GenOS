//! The UART may disappear or stop responding while reporting an exception.
//! Bounded polling lets the caller abandon output and finish fault handling.
pub const TRANSMIT_POLLS: usize = 16_384;

pub fn wait_ready(mut ready: impl FnMut() -> bool) -> bool {
    for _ in 0..TRANSMIT_POLLS {
        if ready() {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stuck_uart_exhausts_exact_budget() {
        let mut calls = 0;
        assert!(!wait_ready(|| {
            calls += 1;
            false
        }));
        assert_eq!(calls, TRANSMIT_POLLS);
    }
    #[test]
    fn readiness_finishes_without_consuming_the_remaining_budget() {
        let mut calls = 0;
        assert!(wait_ready(|| {
            calls += 1;
            calls == 3
        }));
        assert_eq!(calls, 3);
    }
}
