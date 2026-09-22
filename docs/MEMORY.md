# Managed-frame memory contract

## Firmware handoff admission

The loader decodes UEFI version-1 descriptor prefixes from bytes using the
firmware's returned stride. Extended and unaligned strides are supported without
constructing potentially misaligned descriptor references. Empty/truncated maps,
unknown descriptor versions, more than 256 descriptors, zero/overflowing or
unaligned ranges, and any overlapping ranges are rejected. Unknown memory types,
runtime-attributed descriptors and boot-services memory remain reserved. No
descriptor is silently discarded to fit the BootInfo capacity.

Before architecture tables or allocation initialize, the kernel validates the
versioned BootInfo, command-line bounds/UTF-8, complete map, framebuffer extent,
and retained coverage of the linker-defined kernel image, BootInfo object and
initrd. Coverage may span adjacent descriptors in an unsorted map, but may not
cross a gap or usable-memory region. The allocator checks map validity again.
`BOOT_MEMORY_MAP_VALIDATED` means these admission checks passed. Rejected input
halts with `BOOT_MEMORY_MAP_REJECTED` in the loader or `BOOT_INFO_REJECTED` in
the kernel; the loader's fatal path never calls boot services after exit.

The handoff still trusts the loader to provide a mapped, aligned, live Rust
BootInfo object with valid enum representations and a correctly loaded kernel.
This is not validation of an arbitrary pointer or a hostile firmware implementation.
The pinned `uefi` 0.36.1 helper owns map allocation and retries ExitBootServices
once for a stale key; it does not provide an exhaustive map-growth/retry fault
campaign. Those firmware interactions still need their own F1.1 fault campaign. Kernel ELF
loading now has a bounded checked plan; see [KERNEL_ELF.md](KERNEL_ELF.md). The ABI layout and version are unchanged.

`cargo test -p genos_abi` tests malformed map bytes and handoff bounds.
`python3 tools/test_boot_memory.py` builds immutable source fixtures and tests
six actual VM rejection paths; each retains the patch, image/environment hashes,
serial log and pass/failure manifest under `build/boot-memory-evidence/`.

## Allocator execution ownership

The single admitted BSP owns the physical allocator. Initialization uses static
storage with local interrupts disabled; subsequent allocation, release, snapshots
and injection configuration use a scoped IRQ-masked section. Nested sections
restore the incoming IF state. No allocator reference escapes the section. This
is not an SMP lock, and NMI/fatal handlers must not enter the allocator.

## Allocation and release

Every physical grant returned by `memory::alloc_frame(owner, kind)` is zero-filled before the
caller can publish it as a page table, user image page or user stack page. The
paging adapter relies on this guarantee instead of zeroing the same page again.
ELF bytes are then copied into zeroed pages, leaving uncovered bytes and stack
pages initialized.

Release validates alignment, managed-region membership and the live allocation
bit before touching the physical page. It performs 4096 non-elidable volatile
byte stores while the grant is still allocated, then publishes the free bit.
Malformed, unissued and duplicate releases perform no page writes. If a release
preparation callback fails, the allocation remains live. Physical write faults
halt rather than granting an incompletely scrubbed page to another caller.

The [frame-grant contract](FRAME_GRANTS.md) now enforces owner/allocation identity,
stale-generation denial and one pinned user mapping per grant. Teardown validates
its owner tree before mutation, unlinks mappings before release, and refuses active
or stale roots. PCID/global retention is disabled and read back during bootstrap.
The physical release function remains unsafe because raw references and devices
must still be retired by the caller. Kernel physical aliases, explicit sharing,
device/DMA pins and SMP need further work. Zeroing RAM does not promise cache
erasure, physical remanence protection or destruction of unrelated kernel copies.

## Terminal diagnostics

`mem` opens the reserved read-only `/MEMORY.STATUS` file through the ordinary
userspace file capability interface. No syscall or ABI version changes. It reports
managed/live/free frames, peak live frames, successful allocations/releases,
allocator exhaustion, rejected releases, failed release preparation, region count
and consistency. Counts describe managed 4096-byte frames, including page tables;
they are not the entire machine's memory or per-process resident memory.

The node is reserved during startup so a full user directory cannot prevent its
creation. Every authorized open captures one IRQ-protected allocator sample and
attaches a complete, immutable report to that exact read-only file capability.
Partial reads and the handle's reported size keep referring to that sample even
when another process opens the file or memory usage changes. Reopening captures
new values; an existing handle never becomes a live view. Writes/removal through
user capabilities remain denied. The diagnostic is outside `/USER` and is never
persisted. The older path-read syscall has no open lifetime and receives one fresh
sample per call; repeated path reads do not share a snapshot.

Snapshot bytes remain inside the owning process's bounded kernel bookkeeping:
at most four reports of at most 512 bytes each, using the existing four-file
handle budget. There is no heap/frame allocation and no additional userspace ABI.
An oversized report, exhausted file/snapshot table or failed report generation
fails the open without publishing a partial snapshot or leaving a registered
handle. Close, exit, fault, kill and resource revocation release snapshot ownership.
The ordinary typed handle registry and pending-request identity checks still
govern access; stale handles cannot read a reopened report. Operation counters
saturate; deliberate pre-allocation injection is not counted as bitmap exhaustion.
This completes snapshot coherence only: per-owner frame accounting and general
memory-pressure policy remain separate F3.5 work.

Consistency checks cover region ordering/alignment, bitmap capacity, unused bits,
live population and high-water bounds. The managed-address limit is 8 GiB/64 regions. A separate 8192-live-grant
limit bounds ownership metadata and dynamic pages; see the explicit tradeoff in
[FRAME_GRANTS.md](FRAME_GRANTS.md).
Snapshot scans are bounded by the static bitmap capacity and run only at explicit
initialization/diagnostic/validation points, not on every frame operation.

## Verification

`cargo test -p kernel --lib file_snapshot` exercises interleaved opens, every
partial-read size from 1 to 512 bytes, report-buffer mutation, EOF/zero-capacity and
overflowing offsets, stale/revoked/wrong-kind/mutable handles, independent process
tables, bounded exhaustion, duplicate/oversized attachment and cleanup. These
host tests exercise the production snapshot table and handle registry; kernel
copy-out and scheduling still require integration evidence.

`python3 tools/test_memory_snapshot.py` provides that targeted Ring 3 probe using
an immutable archive of committed source. It replaces only the normal shell with
`tools/fixtures/memory_snapshot.rs`, opens a report, reads its prefix, launches a
held process to change live-frame use, opens a second report, and verifies the
first report's remaining bytes, original size and EOF against a baseline witness.
It also verifies a changed second sample, stale-handle denial, kill/reap returning
to the live-frame baseline, and two cycles filling/reusing the four-file budget.
The fixture parks on the input syscall afterward; success requires the normal
shell-ready marker. This adds no startup work or bytes to the ordinary validation
shell. Source patch, image and serial hashes, environment, allocator samples and
pass/failure manifest are retained under `build/memory-snapshot-evidence/`.

`cargo test -p kernel --lib physmem` exercises fragmented ownership, invalid and
duplicate release without invoking memory preparation, failed preparation retaining
ownership, exact-page scrubbing, counters and corrupted metadata. The QEMU command
`cargo xtask test-memory` requires all four records:

- `MEMORY_HYGIENE_READY bytes=4096 invalid_free=denied reused=zero`
- `IRQ_CRITICAL_SECTION_READY nested=preserved outer=restored`
- `MEMORY_ROLLBACK_READY allocation_points=10 leaked_frames=0`
- `FRAME_OWNERSHIP_READY stale=denied foreign=denied alias=denied pinned=denied reclaimed=true`

The first probe poisons a real page, checks invalid releases preserve its bytes,
inspects zero bytes immediately after valid release, and reallocates that same
page. The second uses actual IF state across nested sections. The existing process
construction failure matrix still restores the live-frame baseline at every cutoff.
`cargo xtask test-release` also invokes `mem` and denies writing its status file in
both normal profiles.
