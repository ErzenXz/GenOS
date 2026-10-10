//! Owned state with non-escaping, non-waiting exclusive access.
//!
//! This primitive does not mask IRQs, establish device ownership, prevent DMA or
//! make a hardware subsystem SMP-ready. An interrupted caller is never waited on.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Busy;

pub struct ScopedCell<T> {
    borrowed: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: the only value access first acquires the flag exclusively, and its
// mutable borrow ends before a release store. Contenders never dereference it.
// Shared callers can transfer exclusive T access between threads, requiring
// T: Send, but never share &T concurrently, so T: Sync is unnecessary. Fields
// are private; the higher-ranked callback cannot leak a reference to the value.
unsafe impl<T: Send> Sync for ScopedCell<T> {}

impl<T> ScopedCell<T> {
    pub const fn new(value: T) -> Self {
        Self {
            borrowed: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    /// Return Busy before value access on contention or synchronous reentry.
    /// There is no spin, IRQ change, poisoning or transaction rollback. Host
    /// unwinding releases the borrow but may leave a logically changed value.
    ///
    /// ```compile_fail
    /// use kernel::scoped_cell::ScopedCell;
    /// let cell = ScopedCell::new(7);
    /// let escaped = cell.try_with(|value| value);
    /// ```
    ///
    /// ```compile_fail
    /// use kernel::scoped_cell::ScopedCell;
    /// use std::rc::Rc;
    /// fn require_sync<T: Sync>() {}
    /// require_sync::<ScopedCell<Rc<()>>>();
    /// ```
    pub fn try_with<R>(&self, operation: impl for<'a> FnOnce(&'a mut T) -> R) -> Result<R, Busy> {
        self.borrowed
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| Busy)?;
        let _borrow = Borrow(&self.borrowed);
        // SAFETY: the successful acquire grants this callback exclusive value
        // access. Its borrow ends before Borrow releases the flag, including
        // unwinding. &self keeps storage live; no other value accessor exists.
        Ok(operation(unsafe { &mut *self.value.get() }))
    }
}

struct Borrow<'a>(&'a AtomicBool);

impl Drop for Borrow<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn mutations_and_owned_results_survive_the_borrow() {
        let cell = ScopedCell::new(std::string::String::from("owned"));
        assert_eq!(
            cell.try_with(|value| {
                value.push('!');
                value.clone()
            }),
            Ok(std::string::String::from("owned!"))
        );
        assert_eq!(cell.try_with(|value| value.len()), Ok(6));
    }

    #[test]
    fn reentry_refuses_before_calling_or_mutating_the_value() {
        let cell = ScopedCell::new(0);
        cell.try_with(|value| {
            *value = 11;
            assert_eq!(
                cell.try_with(|_| panic!("nested callback")),
                Err::<(), _>(Busy)
            );
            assert_eq!(*value, 11);
            *value = 12;
        })
        .unwrap();
        assert_eq!(cell.try_with(|value| *value), Ok(12));
    }

    #[test]
    fn unwinding_releases_exclusion_without_rolling_back_mutation() {
        let cell = ScopedCell::new(0);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cell.try_with(|value| {
                *value = 17;
                panic!("callback failure");
            })
            .unwrap();
        }));
        assert!(result.is_err());
        assert_eq!(cell.try_with(|value| *value), Ok(17));
    }

    #[test]
    fn repeated_scopes_retire_every_prior_borrow() {
        let cell = ScopedCell::new([0; 8]);
        for round in 0..1024 {
            assert_eq!(
                cell.try_with(|value| {
                    assert_eq!(value[0], round);
                    value.fill(round + 1);
                    *value
                }),
                Ok([round + 1; 8])
            );
        }
    }

    #[test]
    fn independent_cells_can_be_borrowed_together() {
        let first = ScopedCell::new(3);
        let second = ScopedCell::new(4);
        assert_eq!(
            first.try_with(|a| second.try_with(|b| {
                *a += *b;
                *a
            })),
            Ok(Ok(7))
        );
    }

    #[test]
    fn contender_never_enters_while_another_thread_holds_the_borrow() {
        let cell = Arc::new(ScopedCell::new([0; 32]));
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let child_cell = cell.clone();
        let child_entered = entered.clone();
        let child_release = release.clone();
        let child = std::thread::spawn(move || {
            child_cell.try_with(|value| {
                value.fill(23);
                child_entered.wait();
                child_release.wait();
                assert_eq!(*value, [23; 32]);
            })
        });
        entered.wait();
        for _ in 0..64 {
            assert_eq!(
                cell.try_with(|_| panic!("contending access")),
                Err::<(), _>(Busy)
            );
        }
        release.wait();
        assert_eq!(child.join().unwrap(), Ok(()));
        assert_eq!(cell.try_with(|value| *value), Ok([23; 32]));
    }

    #[test]
    fn send_but_non_sync_values_handoff_exclusively_between_threads() {
        let cell = Arc::new(ScopedCell::new(core::cell::Cell::new(0)));
        for round in 0..8 {
            let child_cell = cell.clone();
            std::thread::spawn(move || {
                child_cell
                    .try_with(|value| {
                        assert_eq!(value.get(), round);
                        value.set(round + 1);
                    })
                    .unwrap()
            })
            .join()
            .unwrap();
        }
        assert_eq!(cell.try_with(|value| value.get()), Ok(8));
    }
}
