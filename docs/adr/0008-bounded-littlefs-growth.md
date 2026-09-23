# ADR-0008: Grow persistent storage through bounded littlefs reuse

- **Status:** Proposed; selected direction for the next implementation, not a claim of integration or release acceptance.
- **Date:** 2026-09-22
- **Decision owners:** storage/runtime maintainers
- **Related roadmap:** S2.1; implementation and migration remain S2.2/S2.3, gated by S1 and F3/F4
- **Supersedes:** None

GenOS needs useful files and directories without importing an entire operating
system or inventing every recovery mechanism again. Select the independently
developed littlefs v2 format and its bounded C implementation for the next
storage provider. Keep GFS2 v3 readable and leave the existing normal volume
unchanged until explicit export, migration and rollback tests pass. This choice
accepts a reviewed C boundary and imperfect random-overwrite performance in
exchange for reusable recovery machinery and fixed memory buffers.

## Workloads and constraints

The target is a small single-user console on the frozen single-BSP VM, not a
general database server, flash controller, or multi-host filesystem. Selection
uses these concrete workloads; passing their benchmarks is still implementation
work, not evidence supplied by this ADR:

| Workload | Useful first target | What distinguishes the alternatives |
| --- | --- | --- |
| Source, configuration and command output | 1024 entries, mixed 0–4 KiB files, complete ASCII paths up to 255 bytes | Small-file metadata amplification, traversal cost, bounded enumeration |
| Executables and archives | Sequential create/read of 1 MiB files in 4 KiB chunks | Avoid keeping each file or the whole volume in RAM; sustained transfer |
| Log/output stream | Append 512 B and 4 KiB chunks, explicit durable checkpoints | Incremental work and honest acknowledgement boundaries |
| Editor save | Write a replacement 1 MiB file, sync, atomically rename | Space for old/new data and interruption-safe namespace publication |
| Random update | 1000 seeded 512 B overwrites at beginning/middle/end of a 1 MiB file | A deliberate adverse case; retained bytes written per useful byte |
| Namespace churn | Create/remove/rename nested entries, 50% and 90% physical occupancy | Reclamation, full-volume behavior, interrupted recovery |

The candidate uses a **new disposable 64 MiB volume**, 4 KiB logical allocation
blocks and 512-byte read/program granularity. Initial policy caps are 1 MiB per
file, 1024 total entries, 255-byte complete paths and 32 MiB aggregate user data.
These are product quotas, not littlefs format limits. Existing GFS2's 8 MiB image
and 512-byte files are not enlarged in place. No proposed implementation may
copy 1 MiB onto a kernel stack, preload all file payloads into `RamVfs`, allocate
without owner accounting, or perform filesystem work in an interrupt handler.

## Comparison and decision

The space and I/O estimates below are analytical lower bounds or explicit
candidate designs. They are not benchmark measurements.

| Candidate | Space/write amplification for these workloads | RAM and recovery | Reuse, licence and tooling cost | Decision |
| --- | --- | --- | --- | --- |
| Enlarge current dual full snapshots | Two complete live datasets; with 32 MiB live, at least 64 MiB before metadata. A 512 B edit writes roughly 32 MiB: at least 65,536× payload amplification | A streamed encoder could bound RAM, but existing code holds snapshot/VFS copies. Mount must validate complete snapshots, so roughly twice live bytes read | Smallest immediate code change; preserves our Rust parser/tools. New format/version and migration still needed | Reject for growth; retain bounded legacy reader |
| Custom copy-on-write extents plus dual metadata snapshots | Data writes become incremental. A candidate 320 B record × 1024 entries needs 320 KiB per metadata snapshot before extent overflow tables; a 512 B edit still writes at least 640× its payload if metadata is copied whole. More than one extent per file raises that bound | Allocation bitmap is 2 KiB at 64 MiB/4 KiB. Metadata validation, extent reference checking and recovery require new bounded algorithms | No new dependency, but GenOS owns all allocator, stale extent, orphan and interrupted reclamation bugs; independent checker/repair/export must be built twice | Reject as first growth step; reconsider only with measured reason to replace reuse |
| littlefs v2 with static buffers | Shared metadata pairs and inline small files reduce per-file overhead; data uses copy-on-write CTZ chains. Appends fit well, but overwriting near a chain's beginning can copy a large suffix. Space must cover old/new blocks during a commit | Explicit read/program/file caches and lookahead; no whole-volume RAM map required. Recovery/traversal is bounded by volume and namespace quotas, but may scan metadata/allocation state; no constant mount-time claim | Existing C implementation, format specification, emulated block device and interruption tests. Requires checked callbacks, compiler integration and independent export/inspection. BSD-3-Clause source | **Select**, with adverse random-write and full-volume gates |
| FatFs/FAT32 | Good sequential access and broad interchange; in-place FAT/directory updates avoid double full-data copies but do not provide the required interrupted-mutation contract by themselves | Configurable small caches; a damaged allocation chain can require an external checker | Small C port and established host tools; permissive FatFs terms. Adding our own transactional layer would reintroduce allocator/recovery design | Reserve for exchange media; do not use as the primary persistent namespace |
| ext4 implementation reuse | Extents and metadata journal handle general workloads; journal sizing, block groups and feature negotiation add overhead on a small volume | More complex journal replay and consistency state than this target needs; no measured GenOS RAM/latency budget | Strong ecosystem and recovery tooling, but a larger implementation/feature surface. Inspected lwext4 repository has GPL-2.0 default terms with per-file exceptions, so it cannot be treated as an unnoticed MIT-compatible code drop | Reject for the first console volume; this is scope/maintenance judgement, not a claim that the format is unreliable |

The littlefs design deliberately combines small metadata logs with copy-on-write
data and bounded-memory allocation; its flash origins also mean wear management
is extra work on our sector device. These properties motivate, but do not prove,
the selection. [Upstream design](https://github.com/littlefs-project/littlefs/blob/6cb4e86540eca0d9ba62500a298385c9d863c8be/DESIGN.md).

FatFs explicitly documents interrupted-write critical sections and possible
allocation/file damage; a small footprint alone does not satisfy our recovery
contract. [FatFs application note](https://elm-chan.org/fsw/ff/doc/appnote.html).
ext4's metadata journal and checksummed transaction rules illustrate the extra
replay/feature surface we would take on. [ext4 journal documentation](https://docs.kernel.org/filesystems/ext4/journal.html).
The inspected reuse candidate's licensing must be assessed from its actual
files, not a remembered project description. [lwext4 licence](https://github.com/gkostka/lwext4/blob/master/LICENSE).

## Bounded integration contract

Pin upstream **v2.11.3**, commit
`6cb4e86540eca0d9ba62500a298385c9d863c8be`, before vendoring. The tag and branch
identity were checked on 2026-09-22; dependency updates require a separate
compatibility/fault-corpus review. Keep upstream copyright, licence and changes
identifiable. The selected source uses BSD-3-Clause; retaining its notices and
disclaimer is part of vendoring, not something implied by GenOS's MIT label.
[Pinned licence](https://github.com/littlefs-project/littlefs/blob/6cb4e86540eca0d9ba62500a298385c9d863c8be/LICENSE.md).

One storage provider owns the mounted `lfs_t`, device, static caches and open-file
objects. Rust capabilities retain process ownership, rights and stale-handle
checks; no C pointer, littlefs file number or disk offset becomes user authority.
Only checked, owned kernel buffers cross the wrapper. The provider does not
re-enter itself from callbacks or call the scheduler with a borrowed C object.
Compile with `LFS_NO_MALLOC`; no hidden libc allocator is available.

Candidate buffers are a 512 B read cache, 512 B program cache, a 128 B lookahead
and sixteen 512 B file caches: **9344 B** before structures and wrapper state.
With `LFS_NO_MALLOC`, open each file through `lfs_file_opencfg` with its own
caller-owned `lfs_file_config.buffer`; keep that configuration and cache alive
until close. A plain `lfs_file_open` cannot supply the required static cache.
Cap the complete provider/open-handle allocation at **64 KiB**, measured by
`size_of`/link-map checks; separate owned I/O buffers are charged to their caller.
Use `metadata_max=4096`, `inline_max=128`, explicit file/name maxima, and a maximum
directory depth of 32. Increasing any cap requires repeat exhaustion and stack
measurements. These settings must be tested with 1024 entries; the directory
quota is not achieved by assuming the existing 32-node RAM VFS already supports
it. `metadata_max=4096` equals the candidate block size, so it does not tighten
the default compaction bound; the 255-byte **complete path**, directory depth,
and total-entry caps must be enforced by the GenOS wrapper, not inferred from
littlefs's per-component `name_max`. The callback/cache options and file sync
API are upstream contracts.
[Pinned configuration/API](https://github.com/littlefs-project/littlefs/blob/6cb4e86540eca0d9ba62500a298385c9d863c8be/lfs.h).

For the sector adapter, erase prepares only a free 4 KiB block; initially use
explicit all-ones writes instead of assuming discard/TRIM or skipping erasure
is safe. Program/read callbacks validate block, offset, alignment and overflow.
The sync callback drains the owned sector cache and issues the qualified ATA
flush. Read/program/sync failures quarantine the provider. An unknown final
publication result stays unknown; attempting a blind C retry is prohibited.
General multi-file transactions are not promised by littlefs or this wrapper.

Count every block callback. Start with a hard 131,072-sector-command operation
budget (64 MiB of sector traffic) and bounded controller waits; exhaustion is an
I/O failure followed by quarantine, not success. This is a safety bound, not a
latency target. A later asynchronous provider must retain ownership and stage
cancellation explicitly before introducing yields into C calls. Admission must
reserve room for a full maximum replacement plus metadata before acknowledging
a save; raw free-space estimates do not replace ENOSPC/rollback tests.

## Visibility, acknowledgement and cleanup

Preserve the existing distinction between acknowledged and recovered snapshots
even though littlefs does not publish a whole-volume snapshot per syscall.
Successful current-style mutation replies require `lfs_file_sync` or completed
namespace publication followed by device sync. A future buffered stream ABI
must distinguish bytes accepted from bytes durable; it cannot silently weaken
today's acknowledgement. Upstream file writes are persisted by sync/close;
wrapping `lfs_file_write` alone is insufficient. [Upstream usage and semantics](https://github.com/littlefs-project/littlefs/blob/6cb4e86540eca0d9ba62500a298385c9d863c8be/README.md).

Cancellation before admission changes nothing. After synchronous admission, kill
or lost completion cannot establish absence on media. Close, normal exit,
fault and kill must release each wrapper slot/cache exactly once. Error cleanup
must not issue an unrequested retry/commit through `close`; separate disposal
from durable close semantics. A read-only recovery mount never formats or
repairs media automatically. Bad-block resilience does not authenticate data or
establish protection against controller/DMA corruption.

## Migration, downgrade and rollback

GFS2 v3 and littlefs are different formats, not two meanings of version 3. Give
the new provider its own identified partition and versioned GenOS envelope;
unknown versions/features fail closed. Leave the original GFS2 image and its
read-only decoder available. Do not make an old kernel encounter a new format
under a partition type it treats as a fresh blank writable volume.

Migration is an offline export/import into a **separate** new image: enumerate
the independently checked source, preserve path/data bytes and declared case
policy, import with quotas, sync, reopen read-only, and compare a complete
path/kind/length/hash manifest. Publish the selected-image configuration only
after both images and the manifest are retained. Interruption before selection
keeps the old image authoritative; interruption afterward leaves either selected
image independently inspectable. Never rewrite the only source during migration.

Downgrade selects the retained original GFS2 image read-only. New 1 MiB files or
1024-entry namespaces do not fit its limits: reverse conversion must report every
unrepresentable entry and cannot silently truncate/drop it. Rollback restores the
old system/image selection; forward-only user changes require explicit export,
not a false promise of automatic reverse migration. Test missing, corrupt,
partial and mismatched image-selection records before default activation.

## Evidence required by S2.2/S2.3

1. Compile the pinned C source for the freestanding target with static buffers,
   strict diagnostics and reviewed unsafe wrapper sites. Keep compiler identity,
   linked size, stack high-water and owner-budget evidence.
2. Run upstream interruption tests and GenOS callback faults before/after every
   write/sync, including torn/reordered/lost sectors, dishonest flushes, full
   volumes and a second interruption during recovery. Upstream already provides
   power-loss scenarios and an emulated block device; extend their schedules to
   our sector contract. [Power-loss tests](https://github.com/littlefs-project/littlefs/blob/6cb4e86540eca0d9ba62500a298385c9d863c8be/tests/test_powerloss.toml),
   [emulated device](https://github.com/littlefs-project/littlefs/blob/6cb4e86540eca0d9ba62500a298385c9d863c8be/bd/lfs_emubd.h).
3. Compare guest results with an independent read-only host parser and manifests;
   a host tool linked to the same C decoder is useful tooling but is not an
   independent corruption oracle. Retain unreadable inputs.
4. Run every workload above on fixed-seed fresh, 50% and 90% occupied volumes;
   publish sector commands, flushes, useful bytes, metadata/data amplification,
   peak RAM, stack and operation/recovery latency. Random-update regressions must
   be visible rather than hidden in a sequential average. Proposed reference
   targets are under 2 s for normal mount and under 100 ms p99 for small synced
   saves; measure first and revisit explicitly if unattainable.
5. Prove real Ring 3 large-file/stream behavior, stale capability denial,
   cancellation/kill cleanup, full-volume atomic replacement, export/import,
   interrupted migration and downgrade refusal. Keep release mode separate from
   fault fixtures. No GUI or Linux compatibility layer is required for this.

The choice is revisited if the static-memory/stack gates fail, random-overwrite
amplification makes the intended editor workload unusable, or a trustworthy
independent checker costs more than the selected implementation saves. Until
then, implementing a second custom allocator/journal in parallel would divide
the recovery effort without supplying a better measured contract.
