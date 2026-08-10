#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandleKind {
    File,
    EndpointReceive,
    EndpointSend,
    Console,
    Lifecycle,
    Process,
    Socket,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HandleEntry {
    handle: u64,
    kind: HandleKind,
    rights: u64,
}

/// The single authority registry owned by one process. Subsystems may retain
/// metadata for an open object, but a handle is usable only while this table
/// contains the exact value, expected type, and required rights.
pub struct HandleTable<const N: usize> {
    entries: [Option<HandleEntry>; N],
}

impl<const N: usize> HandleTable<N> {
    pub const fn new() -> Self {
        Self { entries: [None; N] }
    }

    pub fn register(&mut self, handle: u64, kind: HandleKind, rights: u64) -> bool {
        if handle == 0
            || self
                .entries
                .iter()
                .flatten()
                .any(|entry| entry.handle == handle)
        {
            return false;
        }
        let Some(slot) = self.entries.iter().position(Option::is_none) else {
            return false;
        };
        self.entries[slot] = Some(HandleEntry {
            handle,
            kind,
            rights,
        });
        true
    }

    pub fn allows(&self, handle: u64, kind: HandleKind, required_rights: u64) -> bool {
        self.entries.iter().flatten().any(|entry| {
            entry.handle == handle
                && entry.kind == kind
                && entry.rights & required_rights == required_rights
        })
    }

    pub fn unregister(&mut self, handle: u64, kind: HandleKind) -> bool {
        let Some(slot) = self.entries.iter().position(|entry| {
            entry.is_some_and(|entry| entry.handle == handle && entry.kind == kind)
        }) else {
            return false;
        };
        self.entries[slot] = None;
        true
    }

    pub fn len(&self) -> usize {
        self.entries.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&mut self) {
        self.entries = [None; N];
    }
}

impl<const N: usize> Default for HandleTable<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{HandleKind, HandleTable};

    #[test]
    fn exact_kind_and_rights_are_required() {
        let mut handles = HandleTable::<4>::new();
        assert!(handles.register(0x41, HandleKind::File, 0b011));
        assert!(handles.allows(0x41, HandleKind::File, 0b001));
        assert!(handles.allows(0x41, HandleKind::File, 0b011));
        assert!(!handles.allows(0x41, HandleKind::File, 0b100));
        assert!(!handles.allows(0x41, HandleKind::EndpointSend, 0b001));
        assert!(!handles.allows(0x42, HandleKind::File, 0b001));
    }

    #[test]
    fn authority_is_local_to_one_caller_table() {
        let mut owner = HandleTable::<2>::new();
        let other = HandleTable::<2>::new();
        assert!(owner.register(0xd101, HandleKind::Process, 1));
        assert!(owner.allows(0xd101, HandleKind::Process, 1));
        assert!(!other.allows(0xd101, HandleKind::Process, 1));
    }

    #[test]
    fn duplicate_full_revoke_and_clear_are_explicit() {
        let mut handles = HandleTable::<2>::new();
        assert!(handles.register(1, HandleKind::Console, 1));
        assert!(!handles.register(1, HandleKind::Lifecycle, 1));
        assert!(handles.register(2, HandleKind::Lifecycle, 1));
        assert!(!handles.register(3, HandleKind::File, 1));
        assert!(!handles.unregister(1, HandleKind::Lifecycle));
        assert!(handles.unregister(1, HandleKind::Console));
        assert_eq!(handles.len(), 1);
        assert!(!handles.allows(1, HandleKind::Console, 1));
        handles.clear();
        assert!(handles.is_empty());
        assert!(!handles.allows(2, HandleKind::Lifecycle, 1));
    }
}
