//! Exact diagnostic records published by the persistent-storage service.
//! Parsing a report grants no authority; the kernel enforces mutation denial.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageMode {
    Writable,
    ReadOnly,
    Unavailable,
    Quarantined,
}

pub fn parse(status: &[u8]) -> Option<StorageMode> {
    match status {
        b"state=healthy" | b"state=recovered" => Some(StorageMode::Writable),
        b"state=readonly" => Some(StorageMode::ReadOnly),
        b"state=error" => Some(StorageMode::Unavailable),
        b"state=readonly\ncommit=not-published\nview=last-acknowledged\nrecovery=remount"
        | b"state=readonly\ncommit=unknown\nview=last-acknowledged\nrecovery=remount" => {
            Some(StorageMode::Quarantined)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_legacy_modes_and_both_quarantine_outcomes() {
        for (record, mode) in [
            (&b"state=healthy"[..], StorageMode::Writable),
            (&b"state=recovered"[..], StorageMode::Writable),
            (&b"state=readonly"[..], StorageMode::ReadOnly),
            (&b"state=error"[..], StorageMode::Unavailable),
            (
                &b"state=readonly\ncommit=not-published\nview=last-acknowledged\nrecovery=remount"
                    [..],
                StorageMode::Quarantined,
            ),
            (
                &b"state=readonly\ncommit=unknown\nview=last-acknowledged\nrecovery=remount"[..],
                StorageMode::Quarantined,
            ),
        ] {
            assert_eq!(parse(record), Some(mode));
        }
    }

    #[test]
    fn rejects_incomplete_records_except_the_complete_legacy_record() {
        for record in [
            &b"state=readonly\ncommit=not-published\nview=last-acknowledged\nrecovery=remount"[..],
            &b"state=readonly\ncommit=unknown\nview=last-acknowledged\nrecovery=remount"[..],
        ] {
            for length in 0..record.len() {
                let expected = (length == b"state=readonly".len()).then_some(StorageMode::ReadOnly);
                assert_eq!(parse(&record[..length]), expected, "prefix {length}");
            }
        }
    }

    #[test]
    fn rejects_unknown_duplicate_inconsistent_and_nontext_records() {
        for record in [
            &b"state=readonly\ncommit=committed\nview=last-acknowledged\nrecovery=remount"[..],
            &b"state=healthy\ncommit=unknown\nview=last-acknowledged\nrecovery=remount"[..],
            &b"state=readonly\ncommit=unknown\ncommit=unknown\nview=last-acknowledged\nrecovery=remount"[..],
            &b"state=readonly\ncommit=unknown\nview=current\nrecovery=remount"[..],
            &b"state=readonly\ncommit=unknown\nview=last-acknowledged\nrecovery=retry"[..],
            &b"state=healthy\n"[..], &b"state=healthy\0"[..], &b"state=healthy\xff"[..],
        ] {
            assert_eq!(parse(record), None);
        }
    }
}
