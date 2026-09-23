# Storage-format decision source check

Research date: **2026-09-23**. Scope: [ADR-0008](../adr/0008-bounded-littlefs-growth.md)
and roadmap S2.1 only. This is an upstream-source review, not implementation,
benchmark, fault-injection, or physical power-loss evidence.

## Verified upstream facts

- `git ls-remote` resolves littlefs tag **v2.11.3** to
  `6cb4e86540eca0d9ba62500a298385c9d863c8be`; the ADR's immutable links
  therefore name the intended source. The [pinned licence](https://github.com/littlefs-project/littlefs/blob/6cb4e86540eca0d9ba62500a298385c9d863c8be/LICENSE.md)
  has BSD-3-Clause conditions, including retention of source notices and a
  binary-distribution notice. The source is not yet vendored into GenOS.
- The [pinned design](https://github.com/littlefs-project/littlefs/blob/6cb4e86540eca0d9ba62500a298385c9d863c8be/DESIGN.md)
  describes two-block metadata logs, copy-on-write file data, and backward CTZ
  chains. The latter permit append without recopying earlier blocks, while an
  edit near the beginning can require copying the following data. This supports
  the ADR's workload comparison, but its space and write-amplification figures
  remain analytical candidate estimates, not GenOS measurements.
- The [pinned API](https://github.com/littlefs-project/littlefs/blob/6cb4e86540eca0d9ba62500a298385c9d863c8be/lfs.h)
  exposes caller-supplied read, program, lookahead and per-file buffers.
  `LFS_NO_MALLOC` requires `lfs_file_opencfg` with a live file buffer; ordinary
  `lfs_file_open` is unavailable or fails without it. `metadata_max` must be at
  most `block_size`, and `inline_max` at most `cache_size`, `attr_max`, and
  `block_size/8`. The proposed 4096/128 limits meet those relationships with
  4096-byte blocks and 512-byte caches, though `metadata_max=4096` is the
  default block-size bound rather than a tighter limit. `name_max` limits one
  path component, not the proposed 255-byte complete path or 1024-entry
  namespace; GenOS must enforce those separately.
- The [pinned README](https://github.com/littlefs-project/littlefs/blob/6cb4e86540eca0d9ba62500a298385c9d863c8be/README.md)
  says file writes become persistent on successful sync or close, while rename
  and remove are power-loss atomic under littlefs's block-device contract. Its
  [callback API](https://github.com/littlefs-project/littlefs/blob/6cb4e86540eca0d9ba62500a298385c9d863c8be/lfs.h)
  requires erased blocks before programming and propagates device errors. The
  [upstream power-loss cases](https://github.com/littlefs-project/littlefs/blob/6cb4e86540eca0d9ba62500a298385c9d863c8be/tests/test_powerloss.toml)
  include partially programmed metadata, and the
  [emulated block device](https://github.com/littlefs-project/littlefs/blob/6cb4e86540eca0d9ba62500a298385c9d863c8be/bd/lfs_emubd.h)
  exists. Neither fact proves GenOS's future sector adapter, ATA flush, controller
  ordering, or dishonest-cache behavior. The ADR correctly makes those separate
  S2.2/S2.3 fault gates.
- [FatFs's own application note](https://elm-chan.org/fsw/ff/doc/appnote.html)
  identifies interrupted FAT updates that can lose or cross-link files; it also
  gives permissive source-retention licence terms. The
  [Linux ext4 journal documentation](https://docs.kernel.org/filesystems/ext4/journal.html)
  describes transaction descriptors, checksums and recovery rules. The
  [inspected lwext4 licence](https://github.com/gkostka/lwext4/blob/master/LICENSE)
  contains GPL-2.0 text and says individual files can differ. Those sources
  support the ADR's comparison as an implementation/reuse judgement, not a
  claim that FAT or ext4 are generally unsuitable.

## Decision status and limits

S2.1's **scoped design criterion** is met by ADR-0008 if a documented selected
direction counts as a decision: it compares concrete workloads, space and write
amplification, RAM, recovery, implementation/reuse/licensing cost, and tooling,
then selects a pinned littlefs candidate. The ADR itself remains **Proposed**;
maintainers can leave S2.1 unchecked if their workflow requires an Accepted ADR.
No littlefs code has been integrated or run in the guest, and none of S2.2's
capacity/streaming or S2.3's migration/rename/backup acceptance follows from
this source check. Before treating the selection as a stable storage contract,
the repository still needs a measured reference workload, reviewed static C
boundary, honest device sync, independent image inspection, repeated fault cuts,
and safe GFS2 read-only migration/rollback evidence.
