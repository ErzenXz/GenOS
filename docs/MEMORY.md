# Managed-frame memory contract

The single admitted BSP owns the physical allocator. Initialization uses static
storage with local interrupts disabled; subsequent allocation, release, snapshots
and injection configuration use a scoped IRQ-masked section. Nested sections
restore the incoming IF state. No allocator reference escapes the section. This
is not an SMP lock, and NMI/fatal handlers must not enter the allocator.

## Allocation and release

Every physical grant returned by `memory::alloc_frame` is zero-filled before the
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

The physical release function is explicitly unsafe: callers must own the grant
and retire references and active mappings before release. The process
teardown path switches to the kernel address space before returning user pages;
page-table rollback releases only its own allocations. A raw frame address is
still not a caller identity token: foreign live-grant release, physical alias
ownership, device/DMA retirement and SMP need further work. Zeroing RAM does not
promise cache erasure, physical remanence protection or destruction of unrelated
kernel copies of data.

## Terminal diagnostics

`mem` opens the reserved read-only `/MEMORY.STATUS` file through the ordinary
userspace file capability interface. No syscall or ABI version changes. It reports
managed/live/free frames, peak live frames, successful allocations/releases,
allocator exhaustion, rejected releases, failed release preparation, region count
and consistency. Counts describe managed 4096-byte frames, including page tables;
they are not the entire machine's memory or per-process resident memory.

The node is reserved during startup so a full user directory cannot prevent its
creation. Its contents are refreshed on an authorized open; writes/removal through
user capabilities are denied. It is outside `/USER` and is never persisted.
The report is a diagnostic snapshot, not a live transaction. Another reader's open
can refresh this shared file while an existing reader consumes it. Each refresh
is generated from one IRQ-protected sample, but readers must not assume a stable
version across concurrent opens. Per-open synthetic-file snapshots remain future
work. Operation counters saturate; deliberate pre-allocation injection is not
counted as bitmap exhaustion.

Consistency checks cover region ordering/alignment, bitmap capacity, unused bits,
live population and high-water bounds. The 8-GiB/64-region limit is unchanged.
Snapshot scans are bounded by the static bitmap capacity and run only at explicit
initialization/diagnostic/validation points, not on every frame operation.

## Verification

`cargo test -p kernel --lib physmem` exercises fragmented ownership, invalid and
duplicate release without invoking memory preparation, failed preparation retaining
ownership, exact-page scrubbing, counters and corrupted metadata. The QEMU command
`cargo xtask test-memory` requires all three records:

- `MEMORY_HYGIENE_READY bytes=4096 invalid_free=denied reused=zero`
- `IRQ_CRITICAL_SECTION_READY nested=preserved outer=restored`
- `MEMORY_ROLLBACK_READY allocation_points=10 leaked_frames=0`

The first probe poisons a real page, checks invalid releases preserve its bytes,
inspects zero bytes immediately after valid release, and reallocates that same
page. The second uses actual IF state across nested sections. The existing process
construction failure matrix still restores the live-frame baseline at every cutoff.
`cargo xtask test-release` also invokes `mem` and denies writing its status file in
both normal profiles.
