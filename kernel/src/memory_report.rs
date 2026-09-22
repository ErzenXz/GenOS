//! Formatting one immutable allocator/grant snapshot, independent of hardware.
use crate::frame_grant::{GRANT_CAPACITY, USER_FRAME_LIMIT};
use crate::physmem::AllocatorStats;
use core::fmt::Write;

#[derive(Clone, Copy)]
pub struct Grants {
    pub live: usize,
    pub kernel: usize,
    pub denied: u64,
    pub exhausted: u64,
    pub quota_denials: u64,
}

pub struct Report {
    bytes: [u8; 512],
    len: usize,
}
impl Write for Report {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        let end = self.len.checked_add(text.len()).ok_or(core::fmt::Error)?;
        if end > self.bytes.len() {
            return Err(core::fmt::Error);
        }
        self.bytes[self.len..end].copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
}
impl Report {
    pub fn new(
        stats: AllocatorStats,
        consistent: bool,
        grants: Grants,
    ) -> Result<Self, core::fmt::Error> {
        let mut report = Self {
            bytes: [0; 512],
            len: 0,
        };
        stats.write_report(&mut report, consistent)?;
        writeln!(
            report,
            "grants_live={} capacity={} kernel={} user={}",
            grants.live,
            GRANT_CAPACITY,
            grants.kernel,
            grants.live.saturating_sub(grants.kernel)
        )?;
        writeln!(
            report,
            "grant_denials={} grant_exhaustion={}",
            grants.denied, grants.exhausted
        )?;
        writeln!(
            report,
            "owner_limit={} quota_denials={}",
            USER_FRAME_LIMIT, grants.quota_denials
        )?;
        Ok(report)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maximum_supported_counts_and_saturated_counters_fit_one_file() {
        let stats = AllocatorStats {
            total: 2_097_152,
            live: 8192,
            free: 2_097_152,
            peak_live: 8192,
            allocations: u64::MAX,
            releases: u64::MAX,
            allocation_failures: u64::MAX,
            invalid_releases: u64::MAX,
            preparation_failures: u64::MAX,
            regions: 64,
        };
        let report = Report::new(
            stats,
            true,
            Grants {
                live: 8192,
                kernel: 8192,
                denied: u64::MAX,
                exhausted: u64::MAX,
                quota_denials: u64::MAX,
            },
        )
        .unwrap();
        let text = core::str::from_utf8(report.as_bytes()).unwrap();
        assert!(text.contains("consistent=yes\n"));
        assert!(text.ends_with("owner_limit=64 quota_denials=18446744073709551615\n"));
        assert!(report.as_bytes().len() <= 512);
        let mut report = report;
        let retained = report.as_bytes().to_vec();
        assert!(report.write_str(&"x".repeat(512)).is_err());
        assert_eq!(report.as_bytes(), retained);
    }
}
