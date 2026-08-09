/// Monotonic request IDs scoped to one process incarnation. Pairing the value
/// with the owning slot and incarnation makes an asynchronous request unique
/// even after a process slot is reclaimed and reused.
pub struct RequestSequence {
    next: u64,
}

impl RequestSequence {
    pub const fn new() -> Self {
        Self { next: 1 }
    }

    pub fn allocate(&mut self) -> Option<u64> {
        let request_id = self.next;
        self.next = request_id.checked_add(1)?;
        Some(request_id)
    }
}

impl Default for RequestSequence {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::RequestSequence;

    #[test]
    fn request_ids_are_nonzero_and_monotonic() {
        let mut sequence = RequestSequence::new();
        assert_eq!(sequence.allocate(), Some(1));
        assert_eq!(sequence.allocate(), Some(2));
        assert_eq!(sequence.allocate(), Some(3));
    }

    #[test]
    fn exhausted_sequence_never_wraps_or_reuses_an_id() {
        let mut sequence = RequestSequence { next: u64::MAX };
        assert_eq!(sequence.allocate(), None);
        assert_eq!(sequence.allocate(), None);
    }
}
