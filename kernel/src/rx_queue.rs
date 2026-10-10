//! Validate an immutable completed RX batch before returning any buffer to DMA.
//!
//! Each descriptor can occur at most once in the captured batch: no buffer is
//! reposted until admission succeeds. This is bookkeeping, not DMA isolation.

pub const RX_HEADER_BYTES: usize = 12;
pub const RX_BUFFER_BYTES: usize = 2048;
pub const RX_MAX_FRAME: usize = 1518;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RxCompletion {
    pub descriptor: u32,
    pub length: u32,
}

impl RxCompletion {
    pub const EMPTY: Self = Self {
        descriptor: 0,
        length: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RxError {
    Busy,
    Quarantined,
    InvalidAdvance,
    InvalidDescriptor,
    DuplicateDescriptor,
    InvalidLength,
}

pub struct RxBatch<const N: usize> {
    used: u16,
    entries: [RxCompletion; N],
    cursor: usize,
    count: usize,
    quarantined: bool,
}

impl<const N: usize> RxBatch<N> {
    /// `used` must match the caller's initial quiescent queue counter.
    pub const fn new(used: u16) -> Self {
        Self {
            used,
            entries: [RxCompletion::EMPTY; N],
            cursor: 0,
            count: 0,
            quarantined: false,
        }
    }

    pub fn used_index(&self) -> u16 {
        self.used
    }
    pub fn pending(&self) -> bool {
        self.cursor < self.count
    }
    pub fn is_quarantined(&self) -> bool {
        self.quarantined
    }
    pub fn quarantine(&mut self) {
        self.quarantined = true;
    }

    /// Admit the complete captured used-ring prefix, with no partial publication.
    /// Do not replace an admitted batch while its buffers are being drained.
    pub fn admit(&mut self, end: u16, entries: &[RxCompletion]) -> Result<(), RxError> {
        if self.quarantined {
            return Err(RxError::Quarantined);
        }
        if self.pending() {
            return Err(RxError::Busy);
        }
        let result = self.validate(end, entries);
        if let Err(error) = result {
            self.quarantine();
            return Err(error);
        }
        self.entries[..entries.len()].copy_from_slice(entries);
        self.cursor = 0;
        self.count = entries.len();
        Ok(())
    }

    fn validate(&self, end: u16, entries: &[RxCompletion]) -> Result<(), RxError> {
        let count = usize::from(end.wrapping_sub(self.used));
        if count > N || entries.len() != count {
            return Err(RxError::InvalidAdvance);
        }
        let mut seen = [false; N];
        for entry in entries {
            let descriptor =
                usize::try_from(entry.descriptor).map_err(|_| RxError::InvalidDescriptor)?;
            let Some(seen) = seen.get_mut(descriptor) else {
                return Err(RxError::InvalidDescriptor);
            };
            if *seen {
                return Err(RxError::DuplicateDescriptor);
            }
            *seen = true;
            let length = usize::try_from(entry.length).map_err(|_| RxError::InvalidLength)?;
            if !(RX_HEADER_BYTES..=RX_BUFFER_BYTES).contains(&length) {
                return Err(RxError::InvalidLength);
            }
        }
        Ok(())
    }

    /// The caller copies this admitted buffer before reposting its descriptor.
    pub fn take(&mut self) -> Option<RxCompletion> {
        if self.quarantined || !self.pending() {
            return None;
        }
        let entry = self.entries[self.cursor];
        self.cursor += 1;
        self.used = self.used.wrapping_add(1);
        Some(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(descriptor: u32) -> RxCompletion {
        RxCompletion {
            descriptor,
            length: 72,
        }
    }

    #[test]
    fn complete_batch_is_admitted_and_drained_in_order() {
        let mut batch = RxBatch::<8>::new(0);
        let entries = [entry(7), entry(2), entry(0)];
        batch.admit(3, &entries).unwrap();
        assert_eq!(batch.used_index(), 0);
        for (index, expected) in entries.into_iter().enumerate() {
            assert_eq!(batch.take(), Some(expected));
            assert_eq!(batch.used_index(), index as u16 + 1);
        }
        assert!(!batch.pending());
        assert_eq!(batch.take(), None);
    }

    #[test]
    fn duplicate_anywhere_quarantines_before_the_first_take() {
        for left in 0..8 {
            for right in left + 1..8 {
                let mut entries = core::array::from_fn::<_, 8, _>(|id| entry(id as u32));
                entries[right] = entries[left];
                let mut batch = RxBatch::<8>::new(0);
                assert_eq!(batch.admit(8, &entries), Err(RxError::DuplicateDescriptor));
                assert_eq!(batch.used_index(), 0);
                assert_eq!(batch.take(), None);
                assert_eq!(batch.admit(1, &[entry(0)]), Err(RxError::Quarantined));
            }
        }
    }

    #[test]
    fn malformed_id_or_length_rejects_even_a_valid_prefix() {
        for invalid in [
            RxCompletion {
                descriptor: 8,
                length: 72,
            },
            RxCompletion {
                descriptor: u32::MAX,
                length: 72,
            },
            RxCompletion {
                descriptor: 1,
                length: 0,
            },
            RxCompletion {
                descriptor: 1,
                length: 11,
            },
            RxCompletion {
                descriptor: 1,
                length: 2049,
            },
            RxCompletion {
                descriptor: 1,
                length: u32::MAX,
            },
        ] {
            let mut batch = RxBatch::<8>::new(0);
            assert!(batch.admit(2, &[entry(0), invalid]).is_err());
            assert!(batch.is_quarantined());
            assert_eq!(batch.used_index(), 0);
            assert_eq!(batch.take(), None);
        }
    }

    #[test]
    fn stale_excess_or_incomplete_snapshot_rejects() {
        for (end, entries) in [
            (9, &[entry(0)][..]),
            (u16::MAX, &[][..]),
            (2, &[entry(0)][..]),
            (0, &[entry(0)][..]),
        ] {
            let mut batch = RxBatch::<8>::new(0);
            assert_eq!(batch.admit(end, entries), Err(RxError::InvalidAdvance));
            assert_eq!(batch.take(), None);
        }
    }

    #[test]
    fn batch_cannot_be_replaced_while_a_completed_buffer_is_owned() {
        let mut batch = RxBatch::<8>::new(0);
        batch.admit(2, &[entry(0), entry(1)]).unwrap();
        assert_eq!(batch.take(), Some(entry(0)));
        assert_eq!(batch.admit(3, &[entry(1), entry(0)]), Err(RxError::Busy));
        assert!(!batch.is_quarantined());
        assert_eq!(batch.take(), Some(entry(1)));
        // Descriptor zero was reposted after its first take; a new snapshot
        // may legitimately report its next completion.
        batch.admit(3, &[entry(0)]).unwrap();
        assert_eq!(batch.take(), Some(entry(0)));
    }

    #[test]
    fn empty_and_wrapping_batches_preserve_counters() {
        let mut batch = RxBatch::<8>::new(u16::MAX);
        batch.admit(u16::MAX, &[]).unwrap();
        assert_eq!(batch.take(), None);
        batch.admit(1, &[entry(0), entry(7)]).unwrap();
        assert_eq!(batch.take(), Some(entry(0)));
        assert_eq!(batch.used_index(), 0);
        assert_eq!(batch.take(), Some(entry(7)));
        assert_eq!(batch.used_index(), 1);
        batch.quarantine();
        assert_eq!(batch.take(), None);
        assert_eq!(batch.admit(2, &[entry(0)]), Err(RxError::Quarantined));
    }

    #[test]
    fn buffer_extent_admission_does_not_change_oversized_packet_drop_policy() {
        let mut batch = RxBatch::<8>::new(0);
        let entries = [12, 1530, 1531, 2048].map(|length| RxCompletion {
            descriptor: 0,
            length,
        });
        for (index, mut entry) in entries.into_iter().enumerate() {
            entry.descriptor = index as u32;
            batch.admit(index as u16 + 1, &[entry]).unwrap();
            assert_eq!(batch.take(), Some(entry));
            assert!(!batch.is_quarantined());
        }
    }
}
