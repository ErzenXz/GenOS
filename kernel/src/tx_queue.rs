//! Ownership of the single descriptor/buffer used by the split TX queue.
//!
//! A timeout is not completion. Quarantine is irreversible for this queue
//! instance; it supplies no reset, DMA quiescence or buffer-release authority.

pub const TX_RING_SIZE: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Idle,
    Device,
    Quarantined,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TxError {
    Busy,
    Quarantined,
    UnexpectedCompletion,
    InvalidAdvance,
    InvalidDescriptor,
}

pub struct Publication {
    pub available_index: u16,
    pub ring_slot: usize,
}

pub struct TxQueue {
    phase: Phase,
    available: u16,
    used: u16,
}

impl Default for TxQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl TxQueue {
    pub const fn new() -> Self {
        Self {
            phase: Phase::Idle,
            available: 0,
            used: 0,
        }
    }

    /// Reserve descriptor zero before touching its buffer or descriptor.
    /// The caller must publish it once with the normal DMA release ordering.
    pub fn begin(&mut self) -> Result<Publication, TxError> {
        match self.phase {
            Phase::Device => return Err(TxError::Busy),
            Phase::Quarantined => return Err(TxError::Quarantined),
            Phase::Idle => {}
        }
        let ring_slot = self.available as usize % TX_RING_SIZE;
        self.available = self.available.wrapping_add(1);
        self.phase = Phase::Device;
        Ok(Publication {
            available_index: self.available,
            ring_slot,
        })
    }

    pub fn used_index(&self) -> u16 {
        self.used
    }

    pub fn is_quarantined(&self) -> bool {
        self.phase == Phase::Quarantined
    }

    /// Observe the device's used index/ID after the DMA acquire ordering.
    /// Exactly one outstanding descriptor may complete, including index wrap.
    pub fn complete(&mut self, used: u16, descriptor: u32) -> Result<(), TxError> {
        let error = match self.phase {
            Phase::Quarantined => Some(TxError::Quarantined),
            Phase::Idle => Some(TxError::UnexpectedCompletion),
            Phase::Device if used.wrapping_sub(self.used) != 1 => Some(TxError::InvalidAdvance),
            Phase::Device if descriptor != 0 => Some(TxError::InvalidDescriptor),
            Phase::Device => None,
        };
        if let Some(error) = error {
            self.quarantine();
            return Err(error);
        }
        self.used = used;
        self.phase = Phase::Idle;
        Ok(())
    }

    pub fn quarantine(&mut self) {
        self.phase = Phase::Quarantined;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_retires_exactly_one_publication() {
        let mut queue = TxQueue::new();
        let first = queue.begin().unwrap();
        assert_eq!((first.available_index, first.ring_slot), (1, 0));
        assert!(matches!(queue.begin(), Err(TxError::Busy)));
        assert_eq!(queue.used_index(), 0);
        assert_eq!(queue.complete(1, 0), Ok(()));
        let second = queue.begin().unwrap();
        assert_eq!((second.available_index, second.ring_slot), (2, 1));
        assert_eq!(queue.complete(2, 0), Ok(()));
    }

    #[test]
    fn timeout_and_late_completion_cannot_authorize_reuse() {
        let mut queue = TxQueue::new();
        queue.begin().unwrap();
        queue.quarantine();
        assert!(matches!(queue.begin(), Err(TxError::Quarantined)));
        assert_eq!(queue.complete(1, 0), Err(TxError::Quarantined));
        assert_eq!(queue.used_index(), 0);
        assert!(matches!(queue.begin(), Err(TxError::Quarantined)));
    }

    #[test]
    fn wrong_descriptor_does_not_retire_the_buffer() {
        for descriptor in [1, 7, u32::MAX] {
            let mut queue = TxQueue::new();
            queue.begin().unwrap();
            assert_eq!(
                queue.complete(1, descriptor),
                Err(TxError::InvalidDescriptor)
            );
            assert_eq!(queue.used_index(), 0);
            assert!(queue.is_quarantined());
            assert!(queue.begin().is_err());
        }
    }

    #[test]
    fn skipped_stale_and_overadvanced_used_indices_quarantine() {
        for used in [0, 2, 8, u16::MAX] {
            let mut queue = TxQueue::new();
            queue.begin().unwrap();
            assert_eq!(queue.complete(used, 0), Err(TxError::InvalidAdvance));
            assert_eq!(queue.used_index(), 0);
            assert!(queue.is_quarantined());
        }
    }

    #[test]
    fn unsolicited_or_duplicate_completion_cannot_grant_ownership() {
        let mut queue = TxQueue::new();
        assert_eq!(queue.complete(1, 0), Err(TxError::UnexpectedCompletion));
        assert!(queue.begin().is_err());
        let mut queue = TxQueue::new();
        queue.begin().unwrap();
        queue.complete(1, 0).unwrap();
        assert_eq!(queue.complete(1, 0), Err(TxError::UnexpectedCompletion));
        assert!(queue.begin().is_err());
    }

    #[test]
    fn wrapping_indices_retain_single_buffer_ownership() {
        let mut queue = TxQueue {
            phase: Phase::Idle,
            available: u16::MAX,
            used: u16::MAX,
        };
        let publication = queue.begin().unwrap();
        assert_eq!((publication.available_index, publication.ring_slot), (0, 7));
        assert_eq!(queue.complete(0, 0), Ok(()));
        assert_eq!(queue.used_index(), 0);
        let publication = queue.begin().unwrap();
        assert_eq!((publication.available_index, publication.ring_slot), (1, 0));
        assert_eq!(queue.complete(1, 0), Ok(()));
    }
}
