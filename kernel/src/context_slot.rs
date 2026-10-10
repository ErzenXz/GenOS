//! Scoped access to one suspended CPU-local object.
//!
//! This is not an SMP lock. The unsafe adapter must admit one CPU, mask ordinary
//! interrupts around publication/access, and keep emergency entries out of Rust.
//! Atomics make the publication state explicit; they do not establish object
//! lifetime, CPU identity, interrupt policy or the absence of external aliases.

use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    AlreadyActive,
    ReentrantAccess,
}

pub struct ContextSlot<T> {
    current: AtomicPtr<T>,
    borrowed: AtomicBool,
}

impl<T> ContextSlot<T> {
    pub const fn new() -> Self {
        Self {
            current: AtomicPtr::new(core::ptr::null_mut()),
            borrowed: AtomicBool::new(false),
        }
    }

    /// Publish only for the duration of `run`, including unwinding on hosts.
    /// Nested publication returns an error without replacing the current object.
    ///
    /// # Safety
    /// All operations on this slot must execute serially on its one owning CPU.
    /// `object` must remain live and stationary until this call ends. The owner
    /// must suspend Rust access to it for that entire interval; only `with_current`
    /// may create references. Assembly may access fields only outside those Rust
    /// borrows. No external actor may mutate/free the object. The caller supplies
    /// IRQ/emergency exclusion; it must not leave an access callback suspended.
    pub unsafe fn run<R>(&self, object: NonNull<T>, run: impl FnOnce() -> R) -> Result<R, Error> {
        self.current
            .compare_exchange(
                core::ptr::null_mut(),
                object.as_ptr(),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|_| Error::AlreadyActive)?;
        let _publication = Publication { slot: self, object };
        Ok(run())
    }

    /// Access cannot return a reference tied to the temporary mutable borrow.
    /// An empty slot returns `None`; nested access is refused before dereference.
    ///
    /// # Safety
    /// Uphold `run`'s owning-CPU and exclusion contract. A published object must
    /// still be live, with no external Rust reference or assembly access in use.
    /// The callback may not switch contexts or enable ordinary interrupts.
    ///
    /// ```compile_fail
    /// use kernel::context_slot::ContextSlot;
    /// let slot = ContextSlot::<u64>::new();
    /// let escaped = unsafe { slot.with_current(|value| value) };
    /// ```
    pub unsafe fn with_current<R>(
        &self,
        access: impl for<'a> FnOnce(&'a mut T) -> R,
    ) -> Result<Option<R>, Error> {
        self.borrowed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::ReentrantAccess)?;
        let _borrow = Borrow { slot: self };
        let Some(mut object) = NonNull::new(self.current.load(Ordering::Acquire)) else {
            return Ok(None);
        };
        // SAFETY: the adapter guarantees the live, stationary object's exclusive
        // ownership for this scope. The borrow guard refuses a second reference;
        // the higher-ranked callback cannot return a reference to this borrow.
        Ok(Some(access(unsafe { object.as_mut() })))
    }
}

impl<T> Default for ContextSlot<T> {
    fn default() -> Self {
        Self::new()
    }
}

struct Borrow<'a, T> {
    slot: &'a ContextSlot<T>,
}

impl<T> Drop for Borrow<'_, T> {
    fn drop(&mut self) {
        assert!(self.slot.borrowed.swap(false, Ordering::AcqRel));
    }
}

struct Publication<'a, T> {
    slot: &'a ContextSlot<T>,
    object: NonNull<T>,
}

impl<T> Drop for Publication<'_, T> {
    fn drop(&mut self) {
        assert!(!self.slot.borrowed.load(Ordering::Acquire));
        assert_eq!(
            self.slot
                .current
                .swap(core::ptr::null_mut(), Ordering::AcqRel),
            self.object.as_ptr(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // SAFETY for these tests: one host thread serially owns each local slot and
    // stationary object. No owner reference is used until publication ends.
    #[test]
    fn empty_access_does_not_call_the_callback() {
        let slot = ContextSlot::<u64>::new();
        assert_eq!(
            unsafe { slot.with_current(|_| panic!("empty slot")) },
            Ok(None::<()>)
        );
    }

    #[test]
    fn mutation_and_owned_results_survive_the_scope_without_publishing_a_reference() {
        let slot = ContextSlot::new();
        let mut object = (7, std::string::String::from("owned"));
        let pointer = NonNull::from(&mut object);
        let result = unsafe {
            slot.run(pointer, || {
                slot.with_current(|value| {
                    value.0 += 1;
                    value.1.clone()
                })
            })
        };
        assert_eq!(result, Ok(Ok(Some(std::string::String::from("owned")))));
        assert_eq!(object.0, 8);
        assert_eq!(unsafe { slot.with_current(|_| ()) }, Ok(None));
    }

    #[test]
    fn rejected_nested_publication_preserves_the_original_object() {
        let slot = ContextSlot::new();
        let mut first = 11;
        let mut second = 22;
        let first_pointer = NonNull::from(&mut first);
        let second_pointer = NonNull::from(&mut second);
        unsafe {
            slot.run(first_pointer, || {
                assert_eq!(
                    slot.run(second_pointer, || panic!("nested run")),
                    Err::<(), _>(Error::AlreadyActive)
                );
                assert_eq!(slot.with_current(|value| *value), Ok(Some(11)));
            })
        }
        .unwrap();
        assert_eq!((first, second), (11, 22));
    }

    #[test]
    fn nested_access_is_refused_before_creating_a_second_reference() {
        let slot = ContextSlot::new();
        let mut object = 0;
        unsafe {
            slot.run(NonNull::from(&mut object), || {
                slot.with_current(|value| {
                    assert_eq!(
                        slot.with_current(|_| panic!("second borrow")),
                        Err::<Option<()>, _>(Error::ReentrantAccess)
                    );
                    *value = 19;
                })
                .unwrap();
                assert_eq!(slot.with_current(|value| *value), Ok(Some(19)));
            })
        }
        .unwrap();
        assert_eq!(object, 19);
    }

    #[test]
    fn unwinding_releases_both_scopes_and_allows_reuse() {
        let slot = ContextSlot::new();
        let mut object = 1;
        let pointer = NonNull::from(&mut object);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            slot.run(pointer, || {
                slot.with_current(|value| {
                    *value = 2;
                    panic!("callback failed");
                })
            })
            .unwrap()
            .unwrap();
        }));
        assert!(result.is_err());
        assert_eq!(object, 2);
        assert_eq!(unsafe { slot.with_current(|_| ()) }, Ok(None));
        unsafe {
            slot.run(NonNull::from(&mut object), || {
                slot.with_current(|value| *value = 3)
            })
        }
        .unwrap()
        .unwrap();
        assert_eq!(object, 3);
    }

    #[test]
    fn repeated_contexts_never_observe_a_previous_publication() {
        let slot = ContextSlot::new();
        for index in 0..1024 {
            let mut object = index;
            unsafe {
                slot.run(NonNull::from(&mut object), || {
                    slot.with_current(|value| *value += 1)
                })
            }
            .unwrap()
            .unwrap();
            assert_eq!(object, index + 1);
            assert_eq!(unsafe { slot.with_current(|_| ()) }, Ok(None));
        }
    }

    #[test]
    fn assembly_field_pointers_remain_usable_between_scoped_rust_entries() {
        struct Process {
            image: [u64; 8],
            resumes: u64,
        }
        let slot = ContextSlot::new();
        let mut object = Process {
            image: [0; 8],
            resumes: 0,
        };
        let pointer = NonNull::from(&mut object);
        // SAFETY: model the production trampoline: field pointers derive from
        // the publication pointer; raw field access and Rust borrows never overlap.
        unsafe {
            let image = core::ptr::addr_of_mut!((*pointer.as_ptr()).image);
            slot.run(pointer, || {
                for round in 0..16 {
                    image.write([round; 8]);
                    slot.with_current(|process| {
                        assert_eq!(process.image, [round; 8]);
                        process.resumes += 1;
                        process.image[7] += 1;
                    })
                    .unwrap();
                    assert_eq!((*image)[7], round + 1);
                }
            })
            .unwrap();
        }
        assert_eq!(object.resumes, 16);
        assert_eq!(object.image, [15, 15, 15, 15, 15, 15, 15, 16]);
    }
}
