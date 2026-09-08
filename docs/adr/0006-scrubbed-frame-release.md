# ADR-0006: Scrub managed frames before publishing them for reuse

- **Status:** Proposed
- **Date:** 2026-09-08
- **Related roadmap gates:** F3, F5

## Context and decision

Zero-before-user-mapping existed in paging, but returned physical frames retained
their contents. Centralize zero-before-grant and scrub-before-release in the
physical adapter. The host-testable bitmap validates a live grant before invoking
release preparation and changes the free bit only after preparation succeeds.
All runtime adapter access is serialized by scoped local IRQ masking on the BSP.

The allocator itself remains a metadata model, allowing tests to supply backing
memory and rejected callbacks without privileged I/O. Physical release uses
volatile byte stores and a compiler fence so erasure cannot be removed as dead
writes. It does not release an address that failed validation.

## Tradeoffs and remaining work

Uniform erasure costs 4096 bounded stores per returned frame, including page tables.
It avoids a separate sensitive/non-sensitive classification that could omit a user
page. Reclamation is coordinator work, not interrupt-handler work. This is a
conservative correctness choice, not a performance claim. Local IRQ exclusion
covers only the admitted BSP; it is not synchronization for other CPUs or NMIs.

A live bitmap bit does not prove which subsystem owns the grant. Owner tokens,
physical alias tracking and device retirement are still open. See the full
[memory contract](../MEMORY.md) for snapshot semantics and residual limits.

## Compatibility and rollback

There is no userspace ABI, boot ABI, storage or wire-format migration. `mem` uses
an ordinary read-only diagnostic file. Reverting the policy requires restoring
paging's zero-before-publication guarantee at the same time; never remove both
zeroing paths. Reverting release scrubbing reintroduces retained freed-page data.

## Verification

Host tests exercise rejected/failed preparation and exact-page writes. QEMU
poisons, releases, directly inspects and reuses physical memory; checks nested
IRQ restoration; and runs the existing allocation rollback matrix. Normal boot
acceptance reads the memory report and rejects a write to it.
