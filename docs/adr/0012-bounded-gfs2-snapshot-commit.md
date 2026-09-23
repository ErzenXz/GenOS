# ADR-0012: Keep GFS2 dual snapshots as the bounded current volume contract

- **Status:** Proposed; codifies the implemented small-volume decision.
- **Date:** 2026-09-23
- **Decision owners:** storage/runtime maintainers
- **Related roadmap:** F4.5, S1, S2.1–S2.3
- **Supersedes:** None

## Decision

The current 8 MiB reference data disk contains a selected MBR GenOS partition
and two 40-sector `GFS2` v3 snapshot slots. One complete generation is active.
A mutation below `/USER` captures the prior RAM view, changes the VFS,
invalidates the destination slot, writes payload, and publishes a checked
commit header only after the device flush path completes. A successful reply
means the new snapshot was acknowledged under the selected device contract.
A rejected mutation leaves the old RAM/media view. Any write/flush failure
quarantines the volume, discards dirty cache state and restores the last
acknowledged RAM view. If the final publication may have reached media,
remount may select either complete old or complete new state; the caller's
failed operation is never falsely acknowledged as durable.

This chooses a small, fully checkable transactional unit over in-place FAT
mutation or an unfinished journal/extent allocator. It deliberately incurs
whole-snapshot write amplification and 512-byte-file/32-node limits. Those
limits are why [ADR-0008](0008-bounded-littlefs-growth.md) chooses a separate
bounded littlefs candidate for future growth. ADR-0008 does **not** change the
current on-disk bytes or authorize in-place conversion.

## Failure, migration and rollback

Unknown versions, bad checksum/commit, inconsistent parent order, duplicate or
out-of-bound paths, torn generations and unreadable sectors are rejected or
mounted read-only according to the [storage contract](../STORAGE.md). Neither
kernel nor repair tool auto-formats unreadable media. Host repair requires an
offline stable image, one independently trusted generation and a retained
pre-repair copy. A second failed repair must preserve the original source.
Checksums detect modeled corruption, not malicious alteration.

The old GFS2 reader and original image remain available for read-only export
when a new provider is introduced. Migration writes a **separate** image,
compares path/kind/length/hash manifests after reopening, and changes the
selected image only after both images are retained. Downgrade to the old
bundle cannot silently truncate files that exceed its limits. A host stop
is not a guest durability guarantee; a successful flush depends on the
controller/cache assumptions described in [STORAGE.md](../STORAGE.md).

## Evidence and limits

The production-seam 606-image fault corpus, independent host decoder,
mount/repair negative cases and real QEMU create/restore/corrupt/read-only
phases support the scoped current contract. Physical power cuts, dishonest
cache completion, scalable files/streams and new-format migration are not
proved by those tests. The [compatibility inventory](../COMPATIBILITY.md)
contains version and rollback rules; `kernel/src/storage.rs` and
`tools/xtask/src/main.rs` own the current implementation/inspection paths.
