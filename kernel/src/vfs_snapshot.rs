//! Owned rollback storage borrowed for an entire VFS mutation/commit callback.

use crate::vfs::RamVfs;

pub struct VfsSnapshot {
    saved: RamVfs,
}

impl VfsSnapshot {
    pub const fn new() -> Self {
        Self {
            saved: RamVfs::new(),
        }
    }

    /// Capture the complete prior view before invoking the operation. Neither
    /// view may escape, and exclusive self access prevents snapshot replacement
    /// until the callback returns. This does not itself commit or restore:
    /// storage must restore before publishing its failure/quarantine status.
    ///
    /// ```compile_fail
    /// use kernel::{vfs::RamVfs, vfs_snapshot::VfsSnapshot};
    /// let mut snapshot = VfsSnapshot::new();
    /// let mut live = RamVfs::new();
    /// let escaped = snapshot.with_snapshot(&mut live, |_, saved| saved);
    /// ```
    pub fn with_snapshot<R>(
        &mut self,
        live: &mut RamVfs,
        operation: impl for<'a, 'b> FnOnce(&'a mut RamVfs, &'b RamVfs) -> R,
    ) -> R {
        self.saved.copy_from(live);
        operation(live, &self.saved)
    }
}

impl Default for VfsSnapshot {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::{VfsError, MAX_FILE_BYTES, MAX_NODES};

    #[test]
    fn capture_precedes_mutation_and_owned_result_can_leave() {
        let mut live = RamVfs::new();
        live.init_root();
        live.write("/note", b"before").unwrap();
        let mut snapshot = VfsSnapshot::new();
        let result = snapshot.with_snapshot(&mut live, |live, saved| {
            live.write("/note", b"after").unwrap();
            assert_eq!(saved.read("/note"), Ok(&b"before"[..]));
            live.read("/note").unwrap().len()
        });
        assert_eq!(result, 5);
        assert_eq!(live.read("/note"), Ok(&b"after"[..]));
    }

    #[test]
    fn full_capacity_and_file_bytes_restore_without_aliasing() {
        let mut live = RamVfs::new();
        live.init_root();
        for index in 1..MAX_NODES {
            live.write(
                &std::format!("/file{index}"),
                &[index as u8; MAX_FILE_BYTES],
            )
            .unwrap();
        }
        let mut snapshot = VfsSnapshot::new();
        snapshot.with_snapshot(&mut live, |live, saved| {
            live.remove("/file1").unwrap();
            live.truncate("/file2").unwrap();
            live.write("/new", b"transient").unwrap();
            live.copy_from(saved);
        });
        assert_eq!(live.count(), MAX_NODES);
        assert!(live.find("/new").is_none());
        for index in 1..MAX_NODES {
            assert_eq!(
                live.read(&std::format!("/file{index}")),
                Ok(&[index as u8; MAX_FILE_BYTES][..])
            );
        }
    }

    #[test]
    fn failure_status_published_after_restore_survives() {
        let mut live = RamVfs::new();
        live.init_root();
        live.write("/status", b"healthy").unwrap();
        live.mkdir("/user").unwrap();
        let mut snapshot = VfsSnapshot::new();
        snapshot.with_snapshot(&mut live, |live, saved| {
            live.write("/user/new", b"unacknowledged").unwrap();
            live.copy_from(saved);
            live.write("/status", b"readonly unknown").unwrap();
        });
        assert_eq!(live.read("/user/new"), Err(VfsError::NotFound));
        assert_eq!(live.read("/status"), Ok(&b"readonly unknown"[..]));
    }

    #[test]
    fn each_transaction_captures_the_latest_acknowledged_view() {
        let mut live = RamVfs::new();
        live.init_root();
        let mut snapshot = VfsSnapshot::new();
        snapshot.with_snapshot(&mut live, |live, _| {
            live.write("/first", b"committed").unwrap()
        });
        snapshot.with_snapshot(&mut live, |live, saved| {
            live.remove("/first").unwrap();
            live.write("/second", b"rejected").unwrap();
            live.copy_from(saved);
        });
        assert_eq!(live.read("/first"), Ok(&b"committed"[..]));
        assert!(live.find("/second").is_none());
    }

    #[test]
    fn independent_owners_do_not_replace_each_others_snapshot() {
        let mut first = RamVfs::new();
        first.init_root();
        first.write("/a", b"a").unwrap();
        let mut second = RamVfs::new();
        second.init_root();
        second.write("/b", b"b").unwrap();
        VfsSnapshot::new().with_snapshot(&mut first, |first, first_saved| {
            VfsSnapshot::new().with_snapshot(&mut second, |second, second_saved| {
                first.remove("/a").unwrap();
                second.remove("/b").unwrap();
                first.copy_from(first_saved);
                second.copy_from(second_saved);
            });
        });
        assert_eq!(first.read("/a"), Ok(&b"a"[..]));
        assert!(first.find("/b").is_none());
        assert_eq!(second.read("/b"), Ok(&b"b"[..]));
        assert!(second.find("/a").is_none());
    }
}
