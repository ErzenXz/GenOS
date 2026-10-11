//! One irreversible CPU ownership claim on immovable, permanently live storage.
//!
//! This supplies raw allocation provenance, not mappings, DMA coherence,
//! completion/reset authority, access direction or isolation from a device.

use core::{
    cell::{Cell, UnsafeCell},
    marker::PhantomData,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AlreadyClaimed;

pub struct PermanentDma<T: 'static> {
    claimed: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: fields are private and no value reference is exposed. Exactly one
// successful CAS can create a non-cloneable token; the flag is never released.
// All value access is through that token's raw pointer and requires the caller
// to uphold the ordinary unsafe/DMA contracts. T: Send permits exclusive CPU
// ownership to move between threads; concurrent shared &T access is not offered.
unsafe impl<T: Send> Sync for PermanentDma<T> {}

pub struct DmaClaim<T: 'static> {
    storage: &'static PermanentDma<T>,
    _not_sync: PhantomData<Cell<()>>,
}

impl<T> PermanentDma<T> {
    pub const fn new(value: T) -> Self {
        Self {
            claimed: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    /// A static borrow prevents storage movement/destruction after claim, even
    /// if the token is subsequently dropped or a caller unwinds. Neither makes
    /// hardware quiescent or returns the claim to an allocator.
    ///
    /// ```compile_fail
    /// use kernel::permanent_dma::PermanentDma;
    /// let movable = PermanentDma::new(7u8);
    /// let _claim = movable.try_claim();
    /// ```
    ///
    /// ```compile_fail
    /// use kernel::permanent_dma::DmaClaim;
    /// fn require_clone<T: Clone>() {}
    /// require_clone::<DmaClaim<u8>>();
    /// ```
    ///
    /// ```compile_fail
    /// use kernel::permanent_dma::DmaClaim;
    /// fn require_sync<T: Sync>() {}
    /// require_sync::<DmaClaim<u8>>();
    /// ```
    ///
    /// ```compile_fail
    /// use kernel::permanent_dma::PermanentDma;
    /// use std::rc::Rc;
    /// fn require_sync<T: Sync>() {}
    /// require_sync::<PermanentDma<Rc<()>>>();
    /// ```
    pub fn try_claim(&'static self) -> Result<DmaClaim<T>, AlreadyClaimed> {
        self.claimed
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| AlreadyClaimed)?;
        Ok(DmaClaim {
            storage: self,
            _not_sync: PhantomData,
        })
    }
}

impl<T> DmaClaim<T> {
    /// No Rust reference to T is formed. Dereferencing, exposing to a device or
    /// deriving CPU/physical mappings remains the unsafe caller's obligation.
    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.storage.value.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn claim_is_exclusive_and_drop_never_releases_it() {
        static MEMORY: PermanentDma<u32> = PermanentDma::new(7);
        let mut claim = MEMORY.try_claim().unwrap();
        // SAFETY: this test holds the only claim; no device/thread accesses T.
        unsafe {
            claim.as_mut_ptr().write_volatile(9);
            assert_eq!(claim.as_mut_ptr().read_volatile(), 9);
        }
        assert!(matches!(MEMORY.try_claim(), Err(AlreadyClaimed)));
        drop(claim);
        assert!(matches!(MEMORY.try_claim(), Err(AlreadyClaimed)));
    }

    #[test]
    fn unwind_does_not_authorize_reinitialization() {
        static MEMORY: PermanentDma<u8> = PermanentDma::new(0);
        let result = std::panic::catch_unwind(|| {
            let _claim = MEMORY.try_claim().unwrap();
            panic!("initialization failed");
        });
        assert!(result.is_err());
        assert!(matches!(MEMORY.try_claim(), Err(AlreadyClaimed)));
    }

    #[test]
    fn independent_aligned_storage_retains_provenance_and_tail_bytes() {
        #[repr(align(4096))]
        struct Page([u8; 4096]);
        static FIRST: PermanentDma<Page> = PermanentDma::new(Page([0; 4096]));
        static SECOND: PermanentDma<Page> = PermanentDma::new(Page([0; 4096]));
        let mut first = FIRST.try_claim().unwrap();
        let mut second = SECOND.try_claim().unwrap();
        let a = first.as_mut_ptr();
        let b = second.as_mut_ptr();
        assert_eq!(a.addr() % 4096, 0);
        assert_eq!(b.addr() % 4096, 0);
        assert_ne!(a, b);
        // SAFETY: distinct static allocations and their sole claims; offsets
        // are inside initialized arrays. No external device accesses them.
        unsafe {
            core::ptr::addr_of_mut!((*a).0)
                .cast::<u8>()
                .add(4095)
                .write_volatile(31);
            core::ptr::addr_of_mut!((*b).0)
                .cast::<u8>()
                .add(4095)
                .write_volatile(47);
            assert_eq!(
                core::ptr::addr_of!((*a).0)
                    .cast::<u8>()
                    .add(4095)
                    .read_volatile(),
                31
            );
            assert_eq!(
                core::ptr::addr_of!((*b).0)
                    .cast::<u8>()
                    .add(4095)
                    .read_volatile(),
                47
            );
        }
    }

    #[test]
    fn competing_threads_create_exactly_one_send_but_not_sync_claim() {
        static MEMORY: PermanentDma<Cell<u8>> = PermanentDma::new(Cell::new(0));
        let barrier = Arc::new(Barrier::new(4));
        let handles: std::vec::Vec<_> = (0..4)
            .map(|_| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    MEMORY.try_claim().ok()
                })
            })
            .collect();
        let mut winners: std::vec::Vec<_> = handles
            .into_iter()
            .filter_map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(winners.len(), 1);
        let claim = &mut winners[0];
        // SAFETY: all contenders joined, one token exists and no device owns T.
        unsafe {
            (*claim.as_mut_ptr()).set(23);
            assert_eq!((*claim.as_mut_ptr()).get(), 23);
        }
    }

    #[test]
    fn denied_claims_preserve_owned_bytes() {
        static MEMORY: PermanentDma<[u8; 32]> = PermanentDma::new([0; 32]);
        let mut claim = MEMORY.try_claim().unwrap();
        // SAFETY: sole CPU owner, initialized static array, no DMA or aliases.
        unsafe {
            claim.as_mut_ptr().write([0xa5; 32]);
            for _ in 0..64 {
                assert!(matches!(MEMORY.try_claim(), Err(AlreadyClaimed)));
            }
            assert_eq!(claim.as_mut_ptr().read(), [0xa5; 32]);
        }
    }

    #[test]
    fn moving_token_preserves_the_static_storage_address() {
        static MEMORY: PermanentDma<u64> = PermanentDma::new(42);
        fn transfer(claim: DmaClaim<u64>) -> DmaClaim<u64> {
            claim
        }
        let mut claim = MEMORY.try_claim().unwrap();
        let original = claim.as_mut_ptr();
        let mut moved = transfer(claim);
        assert_eq!(original, moved.as_mut_ptr());
    }
}
