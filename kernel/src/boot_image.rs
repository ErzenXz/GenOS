//! Immutable boot-lifetime bytes, published once without integer pointer casts.
//!
//! Readers never wait for an interrupted publisher: they see `None` until its
//! pointer publication completes. This interface grants no executable authority;
//! native-image admission must still validate the bytes before process creation.

use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};

pub struct ImageSlot {
    claimed: AtomicBool,
    pointer: AtomicPtr<u8>,
    length: AtomicUsize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AlreadyPublished;

impl ImageSlot {
    pub const fn new() -> Self {
        Self {
            claimed: AtomicBool::new(false),
            pointer: AtomicPtr::new(ptr::null_mut()),
            length: AtomicUsize::new(0),
        }
    }

    /// Publish borrowed immutable bytes for the entire boot. A second writer
    /// cannot replace or change either field, even while publication is pending.
    ///
    /// ```compile_fail
    /// let slot = kernel::boot_image::ImageSlot::new();
    /// let temporary = [1u8, 2, 3];
    /// slot.publish(&temporary).unwrap();
    /// ```
    pub fn publish(&self, bytes: &'static [u8]) -> Result<(), AlreadyPublished> {
        self.claimed
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
            .map_err(|_| AlreadyPublished)?;
        self.length.store(bytes.len(), Ordering::Relaxed);
        self.pointer
            .store(bytes.as_ptr().cast_mut(), Ordering::Release);
        Ok(())
    }

    pub fn get(&self) -> Option<&'static [u8]> {
        let pointer = self.pointer.load(Ordering::Acquire);
        if pointer.is_null() {
            return None;
        }
        let length = self.length.load(Ordering::Relaxed);
        // SAFETY: only the successful claimant writes these fields, once. The
        // acquired nonnull pointer publishes the preceding length store. Both
        // came from one immutable 'static slice; neither can subsequently change.
        // AtomicPtr preserves its provenance, alignment and allocation identity,
        // including the valid nonnull pointer of an empty slice. This shared
        // reconstruction grants no mutable access and cannot outlive the input.
        Some(unsafe { core::slice::from_raw_parts(pointer.cast_const(), length) })
    }
}

impl Default for ImageSlot {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static FIRST: &[u8] = b"INIT";
    static SECOND: &[u8] = b"a different shell image with a different length";

    #[test]
    fn unavailable_until_published() {
        assert_eq!(ImageSlot::new().get(), None);
    }

    #[test]
    fn preserves_the_original_slice_pointer_and_length() {
        let slot = ImageSlot::new();
        slot.publish(FIRST).unwrap();
        for _ in 0..32 {
            let bytes = slot.get().unwrap();
            assert!(core::ptr::eq(bytes, FIRST));
            assert_eq!(bytes, FIRST);
        }
    }

    #[test]
    fn replacement_cannot_change_published_bytes() {
        let slot = ImageSlot::new();
        slot.publish(FIRST).unwrap();
        assert_eq!(slot.publish(SECOND), Err(AlreadyPublished));
        assert_eq!(slot.publish(&[]), Err(AlreadyPublished));
        assert!(core::ptr::eq(slot.get().unwrap(), FIRST));
    }

    #[test]
    fn empty_slice_is_a_complete_publication() {
        let slot = ImageSlot::new();
        slot.publish(&[]).unwrap();
        assert_eq!(slot.get(), Some(&[][..]));
        assert_eq!(slot.publish(FIRST), Err(AlreadyPublished));
    }

    #[test]
    fn competing_publishers_never_expose_mixed_slice_fields() {
        let slot = ImageSlot::new();
        let start = std::sync::Barrier::new(3);
        std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                start.wait();
                slot.publish(FIRST)
            });
            let second = scope.spawn(|| {
                start.wait();
                slot.publish(SECOND)
            });
            start.wait();
            for _ in 0..32 {
                if let Some(bytes) = slot.get() {
                    assert!(core::ptr::eq(bytes, FIRST) || core::ptr::eq(bytes, SECOND));
                    assert!(bytes == FIRST || bytes == SECOND);
                }
            }
            let first_won = first.join().unwrap().is_ok();
            let second_won = second.join().unwrap().is_ok();
            assert_ne!(first_won, second_won);
            let expected = if first_won { FIRST } else { SECOND };
            assert!(core::ptr::eq(slot.get().unwrap(), expected));
        });
    }
}
