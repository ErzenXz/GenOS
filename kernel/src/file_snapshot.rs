//! Bounded immutable file contents owned by one process's open handles.
//! The ordinary handle registry remains the authority; this table supplies
//! bytes only while the exact file handle still has read-only authority.

use crate::capability::{HandleKind, HandleTable};
use genos_abi::{USER_FILE_RIGHT_MANAGE, USER_FILE_RIGHT_READ, USER_FILE_RIGHT_WRITE};

pub const SNAPSHOT_BYTES: usize = crate::vfs::MAX_FILE_BYTES;

#[derive(Clone, Copy)]
struct Entry {
    handle: u64,
    bytes: [u8; SNAPSHOT_BYTES],
    len: usize,
}

pub struct FileSnapshots<const N: usize> {
    entries: [Option<Entry>; N],
}

fn readable<const H: usize>(handles: &HandleTable<H>, handle: u64) -> bool {
    handles.allows(handle, HandleKind::File, USER_FILE_RIGHT_READ)
        && !handles.allows(handle, HandleKind::File, USER_FILE_RIGHT_WRITE)
        && !handles.allows(handle, HandleKind::File, USER_FILE_RIGHT_MANAGE)
}

impl<const N: usize> FileSnapshots<N> {
    pub const fn new() -> Self {
        Self { entries: [None; N] }
    }

    /// Copy the complete sample or fail without modifying any existing entry.
    /// The caller must undo a newly registered handle if attachment fails.
    pub fn insert<const H: usize>(
        &mut self,
        handle: u64,
        bytes: &[u8],
        handles: &HandleTable<H>,
    ) -> bool {
        if bytes.len() > SNAPSHOT_BYTES || !readable(handles, handle) || self.size(handle).is_some()
        {
            return false;
        }
        let Some(slot) = self.entries.iter_mut().find(|entry| entry.is_none()) else {
            return false;
        };
        let mut entry = Entry {
            handle,
            bytes: [0; SNAPSHOT_BYTES],
            len: bytes.len(),
        };
        entry.bytes[..bytes.len()].copy_from_slice(bytes);
        *slot = Some(entry);
        true
    }

    /// Return a bounded portion of the original sample. An out-of-range
    /// offset is EOF, and a revoked/stale handle never exposes stored bytes.
    pub fn read<const H: usize>(
        &self,
        handle: u64,
        offset: u64,
        capacity: u64,
        handles: &HandleTable<H>,
    ) -> Option<&[u8]> {
        if !readable(handles, handle) {
            return None;
        }
        let entry = self
            .entries
            .iter()
            .flatten()
            .find(|entry| entry.handle == handle)?;
        let start = offset.min(entry.len as u64) as usize;
        let length = capacity.min((entry.len - start) as u64) as usize;
        Some(&entry.bytes[start..start + length])
    }

    /// Metadata for registry-consistency checks, not an authority grant.
    pub fn size(&self, handle: u64) -> Option<usize> {
        self.entries
            .iter()
            .flatten()
            .find(|entry| entry.handle == handle)
            .map(|entry| entry.len)
    }

    pub fn remove(&mut self, handle: u64) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.is_some_and(|entry| entry.handle == handle))
        {
            *entry = None;
        }
    }

    pub fn clear(&mut self) {
        self.entries = [None; N];
    }

    pub fn len(&self) -> usize {
        self.entries.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<const N: usize> Default for FileSnapshots<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_opens_and_partial_reads_keep_their_own_samples() {
        let mut handles = HandleTable::<4>::new();
        let mut snapshots = FileSnapshots::<4>::new();
        assert!(handles.register(0x101, HandleKind::File, USER_FILE_RIGHT_READ));
        let mut report = *b"live=100 free=900";
        assert!(snapshots.insert(0x101, &report, &handles));
        assert_eq!(snapshots.read(0x101, 0, 5, &handles), Some(&b"live="[..]));
        report.copy_from_slice(b"live=200 free=800");
        assert!(handles.register(0x202, HandleKind::File, USER_FILE_RIGHT_READ));
        assert!(snapshots.insert(0x202, &report[9..], &handles));
        report.fill(0);
        assert_eq!(
            snapshots.read(0x101, 5, 99, &handles),
            Some(&b"100 free=900"[..])
        );
        assert_eq!(
            snapshots.read(0x202, 0, 99, &handles),
            Some(&b"free=800"[..])
        );
        assert_eq!(snapshots.size(0x101), Some(report.len()));
        assert_eq!(snapshots.size(0x202), Some(8));
        assert_eq!(
            snapshots.read(0x101, report.len() as u64, 99, &handles),
            Some(&b""[..])
        );
    }

    #[test]
    fn every_partial_read_size_reassembles_the_captured_bytes() {
        let mut handles = HandleTable::<1>::new();
        assert!(handles.register(1, HandleKind::File, USER_FILE_RIGHT_READ));
        let sample = core::array::from_fn::<_, SNAPSHOT_BYTES, _>(|i| (i % 251) as u8);
        let mut snapshots = FileSnapshots::<1>::new();
        assert!(snapshots.insert(1, &sample, &handles));
        for chunk in 1..=SNAPSHOT_BYTES {
            let mut read = std::vec::Vec::new();
            loop {
                let bytes = snapshots
                    .read(1, read.len() as u64, chunk as u64, &handles)
                    .unwrap();
                if bytes.is_empty() {
                    break;
                }
                read.extend_from_slice(bytes);
            }
            assert_eq!(read.as_slice(), &sample);
        }
        assert_eq!(
            snapshots.read(1, u64::MAX, u64::MAX, &handles),
            Some(&b""[..])
        );
        assert_eq!(snapshots.read(1, 0, 0, &handles), Some(&b""[..]));
    }

    #[test]
    fn exhausted_oversized_or_duplicate_attachment_cannot_replace_a_sample() {
        let mut handles = HandleTable::<3>::new();
        for handle in 1..=3 {
            assert!(handles.register(handle, HandleKind::File, USER_FILE_RIGHT_READ));
        }
        let mut snapshots = FileSnapshots::<1>::new();
        assert!(!snapshots.insert(1, &[0; SNAPSHOT_BYTES + 1], &handles));
        assert!(snapshots.is_empty());
        assert!(snapshots.insert(1, b"original", &handles));
        assert!(!snapshots.insert(1, b"replacement", &handles));
        assert!(!snapshots.insert(2, b"overflow", &handles));
        assert_eq!(snapshots.read(1, 0, 99, &handles), Some(&b"original"[..]));
        assert_eq!(snapshots.len(), 1);
        snapshots.remove(99);
        assert_eq!(snapshots.len(), 1);
        snapshots.remove(1);
        assert!(snapshots.insert(3, b"", &handles));
        assert_eq!(snapshots.size(3), Some(0));
        assert!(!FileSnapshots::<0>::new().insert(2, b"no capacity", &handles));
    }

    #[test]
    fn authority_close_reuse_and_owner_cleanup_prevent_stale_reads() {
        let mut handles = HandleTable::<4>::new();
        let mut snapshots = FileSnapshots::<4>::new();
        assert!(!snapshots.insert(0, b"zero", &handles));
        assert!(!snapshots.insert(0x101, b"unregistered", &handles));
        assert!(handles.register(0x101, HandleKind::File, USER_FILE_RIGHT_READ));
        assert!(snapshots.insert(0x101, b"old", &handles));
        assert!(handles.unregister(0x101, HandleKind::File));
        assert_eq!(snapshots.read(0x101, 0, 99, &handles), None);
        snapshots.remove(0x101);
        assert!(handles.register(0x201, HandleKind::File, USER_FILE_RIGHT_READ));
        assert!(snapshots.insert(0x201, b"new", &handles));
        assert_eq!(snapshots.read(0x101, 0, 99, &handles), None);
        assert_eq!(snapshots.read(0x201, 0, 99, &handles), Some(&b"new"[..]));
        handles.clear();
        snapshots.clear();
        assert!(snapshots.is_empty());
        assert!(handles.register(0x201, HandleKind::File, USER_FILE_RIGHT_READ));
        assert_eq!(snapshots.read(0x201, 0, 99, &handles), None);
    }

    #[test]
    fn snapshots_are_read_only_and_owned_by_one_process_table() {
        let mut a = FileSnapshots::<4>::new();
        let mut b = FileSnapshots::<4>::new();
        let mut handles = HandleTable::<4>::new();
        assert!(handles.register(1, HandleKind::File, USER_FILE_RIGHT_READ));
        assert!(a.insert(1, b"process a", &handles));
        assert_eq!(b.read(1, 0, 99, &handles), None);
        assert!(b.insert(1, b"process b", &handles));
        a.clear();
        assert_eq!(b.read(1, 0, 99, &handles), Some(&b"process b"[..]));
        for rights in [
            USER_FILE_RIGHT_WRITE,
            USER_FILE_RIGHT_READ | USER_FILE_RIGHT_WRITE,
            USER_FILE_RIGHT_READ | USER_FILE_RIGHT_MANAGE,
        ] {
            handles.clear();
            assert!(handles.register(1, HandleKind::File, rights));
            assert!(!a.insert(1, b"mutable", &handles));
            assert_eq!(b.read(1, 0, 99, &handles), None);
        }
        handles.clear();
        assert!(handles.register(1, HandleKind::Console, USER_FILE_RIGHT_READ));
        assert!(!a.insert(1, b"wrong kind", &handles));
        assert_eq!(b.read(1, 0, 99, &handles), None);
    }
}
