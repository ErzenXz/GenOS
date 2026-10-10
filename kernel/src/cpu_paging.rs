//! Per-CPU paging bookkeeping, independent of privileged MMU instructions.
//!
//! This object grants no page-table or allocation authority. The adapter must
//! admit one owning CPU, mask local IRQs across hardware changes and publication,
//! read back the actual CR3, and halt on a failed post-change observation. NMI
//! and emergency handlers cannot change this state. Before SMP, each CPU needs
//! its own instance plus the separate acknowledged TLB-retirement protocol.

use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

pub const ROOT_ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;
const EMPTY: u8 = 0;
const INITIALIZING: u8 = 1;
const BOOTSTRAP: u8 = 2;
const SEALED: u8 = 3;

pub struct CpuPaging {
    phase: AtomicU8,
    kernel_root: AtomicU64,
    active_root: AtomicU64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidRoot,
    AlreadyInitialized,
    WrongPhase,
    HardwareMismatch,
}

impl CpuPaging {
    pub const fn new() -> Self {
        Self {
            phase: AtomicU8::new(EMPTY),
            kernel_root: AtomicU64::new(0),
            active_root: AtomicU64::new(0),
        }
    }

    pub fn is_uninitialized(&self) -> bool {
        self.phase.load(Ordering::Acquire) == EMPTY
    }

    /// Publish the permanent supervisor root after the adapter has installed
    /// and read back that exact CR3. Readers do not wait for publication.
    pub fn initialize(&self, root: u64, observed_cr3: u64) -> Result<(), Error> {
        validate_root(root)?;
        if observed_cr3 != root {
            return Err(Error::HardwareMismatch);
        }
        self.phase
            .compare_exchange(EMPTY, INITIALIZING, Ordering::Relaxed, Ordering::Relaxed)
            .map_err(|_| Error::AlreadyInitialized)?;
        self.kernel_root.store(root, Ordering::Relaxed);
        self.active_root.store(root, Ordering::Relaxed);
        self.phase.store(BOOTSTRAP, Ordering::Release);
        Ok(())
    }

    pub fn kernel_root(&self) -> Option<u64> {
        (self.phase.load(Ordering::Acquire) >= BOOTSTRAP)
            .then(|| self.kernel_root.load(Ordering::Relaxed))
    }

    pub fn active_root(&self) -> Option<u64> {
        (self.phase.load(Ordering::Acquire) >= BOOTSTRAP)
            .then(|| self.active_root.load(Ordering::Acquire))
    }

    /// Bootstrap permission/guard mutation is allowed only before sealing and
    /// while the supervisor root is active. This returns an address, not authority.
    pub fn bootstrap_root(&self) -> Option<u64> {
        if self.phase.load(Ordering::Acquire) != BOOTSTRAP {
            return None;
        }
        let root = self.kernel_root.load(Ordering::Relaxed);
        (self.active_root.load(Ordering::Acquire) == root).then_some(root)
    }

    pub fn sealed_root(&self) -> Option<u64> {
        (self.phase.load(Ordering::Acquire) == SEALED)
            .then(|| self.kernel_root.load(Ordering::Relaxed))
    }

    /// Call only after bootstrap permissions and local translations are retired.
    /// The supplied hardware observation cannot be synthesized by this module.
    pub fn seal_kernel(&self, observed_cr3: u64) -> Result<(), Error> {
        let root = self.bootstrap_root().ok_or(Error::WrongPhase)?;
        if observed_cr3 != root {
            return Err(Error::HardwareMismatch);
        }
        self.phase
            .compare_exchange(BOOTSTRAP, SEALED, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::WrongPhase)?;
        Ok(())
    }

    /// Check metadata admission before changing CR3. Live root/grant authority
    /// and local IRQ exclusion remain obligations of the hardware adapter.
    pub fn admit_switch(&self, target: u64) -> Result<(), Error> {
        validate_root(target)?;
        match self.phase.load(Ordering::Acquire) {
            SEALED => Ok(()),
            BOOTSTRAP if self.bootstrap_root() == Some(target) => Ok(()),
            _ => Err(Error::WrongPhase),
        }
    }

    /// Publish only a matching post-switch hardware observation. A refusal
    /// preserves bookkeeping; it cannot undo a hardware operation. The adapter
    /// must halt if this fails after writing CR3, before restoring interrupts.
    pub fn record_switch(&self, target: u64, observed_cr3: u64) -> Result<(), Error> {
        self.admit_switch(target)?;
        if observed_cr3 != target {
            return Err(Error::HardwareMismatch);
        }
        self.active_root.store(target, Ordering::Release);
        Ok(())
    }
}

impl Default for CpuPaging {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_root(root: u64) -> Result<(), Error> {
    if root == 0 || root & !ROOT_ADDRESS_MASK != 0 {
        Err(Error::InvalidRoot)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready() -> CpuPaging {
        let state = CpuPaging::new();
        state.initialize(0x1000, 0x1000).unwrap();
        state.seal_kernel(0x1000).unwrap();
        state
    }

    #[test]
    fn fresh_state_denies_runtime_and_exposes_no_roots() {
        let state = CpuPaging::new();
        assert!(state.is_uninitialized());
        assert_eq!(state.kernel_root(), None);
        assert_eq!(state.active_root(), None);
        assert_eq!(state.bootstrap_root(), None);
        assert_eq!(state.sealed_root(), None);
        assert_eq!(state.seal_kernel(0x1000), Err(Error::WrongPhase));
        assert_eq!(state.admit_switch(0x1000), Err(Error::WrongPhase));
        assert_eq!(state.record_switch(0x1000, 0x1000), Err(Error::WrongPhase));
    }

    #[test]
    fn invalid_or_unobserved_initial_roots_do_not_consume_admission() {
        let state = CpuPaging::new();
        for root in [0, 1, 0x1fff, 1 << 52, u64::MAX] {
            assert_eq!(state.initialize(root, root), Err(Error::InvalidRoot));
            assert!(state.is_uninitialized());
        }
        assert_eq!(
            state.initialize(0x1000, 0x2000),
            Err(Error::HardwareMismatch)
        );
        assert_eq!(
            state.initialize(0x1000, 0x1001),
            Err(Error::HardwareMismatch)
        );
        assert!(state.is_uninitialized());
        state.initialize(0x1000, 0x1000).unwrap();
        assert_eq!(state.bootstrap_root(), Some(0x1000));
    }

    #[test]
    fn supervisor_root_cannot_be_replaced_before_or_after_sealing() {
        let state = CpuPaging::new();
        state.initialize(0x1000, 0x1000).unwrap();
        for sealed in [false, true] {
            if sealed {
                state.seal_kernel(0x1000).unwrap();
            }
            assert_eq!(
                state.initialize(0x2000, 0x2000),
                Err(Error::AlreadyInitialized)
            );
            assert_eq!(state.kernel_root(), Some(0x1000));
            assert_eq!(state.active_root(), Some(0x1000));
        }
    }

    #[test]
    fn bootstrap_cannot_admit_a_private_root_or_unobserved_sealing() {
        let state = CpuPaging::new();
        state.initialize(0x1000, 0x1000).unwrap();
        assert_eq!(state.admit_switch(0x2000), Err(Error::WrongPhase));
        assert_eq!(state.record_switch(0x2000, 0x2000), Err(Error::WrongPhase));
        state.record_switch(0x1000, 0x1000).unwrap();
        assert_eq!(state.seal_kernel(0x2000), Err(Error::HardwareMismatch));
        assert_eq!(state.bootstrap_root(), Some(0x1000));
        assert_eq!(state.sealed_root(), None);
        state.seal_kernel(0x1000).unwrap();
        assert_eq!(state.bootstrap_root(), None);
        assert_eq!(state.seal_kernel(0x1000), Err(Error::WrongPhase));
    }

    #[test]
    fn switches_preserve_permanent_root_and_irreversible_sealing() {
        let state = ready();
        for target in (1..=256)
            .map(|page| page * 4096)
            .chain([ROOT_ADDRESS_MASK, 0x1000])
        {
            state.admit_switch(target).unwrap();
            state.record_switch(target, target).unwrap();
            assert_eq!(state.active_root(), Some(target));
            assert_eq!(state.kernel_root(), Some(0x1000));
            assert_eq!(state.sealed_root(), Some(0x1000));
            assert_eq!(state.bootstrap_root(), None);
        }
    }

    #[test]
    fn invalid_or_mismatched_switches_preserve_the_current_record() {
        let state = ready();
        state.record_switch(0x2000, 0x2000).unwrap();
        for root in [0, 1, 0x1fff, 1 << 52, u64::MAX] {
            assert_eq!(state.admit_switch(root), Err(Error::InvalidRoot));
            assert_eq!(state.record_switch(root, root), Err(Error::InvalidRoot));
            assert_eq!(state.active_root(), Some(0x2000));
        }
        for observed in [0, 0x1000, 0x3001, 1 << 52] {
            assert_eq!(
                state.record_switch(0x3000, observed),
                Err(Error::HardwareMismatch)
            );
            assert_eq!(state.active_root(), Some(0x2000));
        }
    }

    #[test]
    fn competing_initializers_publish_one_complete_root_pair() {
        let state = CpuPaging::new();
        let start = std::sync::Barrier::new(3);
        std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                start.wait();
                state.initialize(0x1000, 0x1000)
            });
            let second = scope.spawn(|| {
                start.wait();
                state.initialize(0x2000, 0x2000)
            });
            start.wait();
            for _ in 0..32 {
                if let Some(root) = state.kernel_root() {
                    assert!(root == 0x1000 || root == 0x2000);
                    assert_eq!(state.active_root(), Some(root));
                }
            }
            let first_won = first.join().unwrap().is_ok();
            let second_won = second.join().unwrap().is_ok();
            assert_ne!(first_won, second_won);
            let winner = if first_won { 0x1000 } else { 0x2000 };
            assert_eq!(state.bootstrap_root(), Some(winner));
            assert_eq!(state.active_root(), Some(winner));
        });
    }
}
