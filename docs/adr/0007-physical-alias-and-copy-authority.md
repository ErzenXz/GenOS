# ADR-0007: Seal physical aliases and copy through live address-space authority

- **Status:** Proposed; implemented on the local foundation branch, not merged
- **Date:** 2026-09-22
- **Related roadmap gates:** F2.1, F2.2, F3.3, F3.5, F5.5
- **Supersedes:** None; extends the bounded policies in ADRs 0003, 0004 and 0006

## Problem and decision

User RX and read-only mappings previously retained a writable supervisor identity
alias. A Ring 0 store could bypass the intended physical permission. User-copy
callers also repeated byte-level translation without one lifetime/permission
interface. The reference profile now admits only an identity-mapped firmware
tree below the user window, freezes supervisor topology before user roots exist,
and makes every retained leaf NX except linked kernel text. Managed RAM receives
4-KiB leaves before runtime. Publishing a read-only/RX user page first makes its
supervisor alias read-only and retires the old permission locally. CR0.WP is
required. User aliases/sharing and in-place user reprotection remain denied.

The inactive owning root is the only place new user mappings can be constructed.
After switching away, teardown unlinks each leaf before restoring its supervisor
alias to writable, retiring its pin, scrubbing and releasing it. Failed mapping
admission leaves the caller's grant unpublished; failed table allocation rolls
back only new edges. Bootstrap permission construction is boot-fatal on failure;
it is not a recoverable runtime allocator operation.

`paging::copy_from_user` and `copy_to_user` own validation and transfer. The pure
`user_copy::Plan` bounds one copy to 4096 bytes/two pages. The hardware adapter
checks the live root, every table owner, effective user/write bits and each exact
leaf pin before any transfer, holding one BSP IRQ exclusion section throughout.
No borrowed user slice or translated address escapes. A rejected later page
cannot cause an earlier partial destination write. ABI-specific buffer bounds
remain at syscall callers. Timer updates of the process header use this interface
too. Fatal hardware faults halt; these interfaces do not promise recovery from
arbitrary corrupted kernel pointers or hostile firmware.

## Ownership, exhaustion and alternatives

Each user owner may hold 64 frames including tables, image and stacks. A 256-entry
ledger reserve is unavailable to user admission. Exhaustion rejects construction
without reclaiming another live process; cleanup returns the exact quota. The
current fixed application image bounds fit this budget. Kernel reserve is ledger
capacity, not a promise of physically available RAM after firmware reservation.
The immutable memory report includes quota denials and fits 512 bytes even with
saturated counters. There is no heap growth, swap, compaction or OOM victim policy.

Keeping writable aliases would leave physical W^X unenforced. Removing all direct
aliases would need temporary mapping slots and a different page-table access
design. Copying by temporarily disabling CR0.WP or leaving SMAP access enabled
would weaken unrelated authority. Sealing the existing shared identity leaves
preserves the current small allocator and avoids those transient bypasses.
It costs bounded bootstrap traversal/splitting, local invalidations, permission
walks during copying and ledger scans; no performance improvement is claimed.

## Compatibility and rollback

ABI18, BootInfo, GFS2 and image layout2 are unchanged. The memory report adds
fields; consumers must parse named fields, not fixed byte offsets or line counts.
Firmware with nonidentity aliases is rejected; it needs a separate supported
profile/loader plan instead of silently bypassing this policy. Revert mapping
sealing, copy admission and teardown restoration together. Reverting sealing
reopens the alias weakness and must also withdraw F2 claims; no disk migration
or format downgrade occurs. General sharing, DMA and SMP require distinct lifetime
protocols before enabling those features.

## Acceptance and review

Host tests cover firmware alias rejection, all copy offsets, malformed ranges,
later-page rejection, per-owner/global pressure, stale grants and report bounds.
Production memory probes cover actual copy denial, quota exhaustion/reuse,
address-space switches and frame reclamation. Separate CPU faults attempt writes
through RX/RO supervisor aliases and execution through an NX direct alias.
Full normal/validation, allocation rollback, protection and process lifecycle
campaigns must run from committed source. Results are recorded separately in
VERIFICATION.md; this decision alone does not assert those campaigns passed.
The remaining unsafe callers, DMA retirement, concurrency and complete CPU
feature-matrix gates are not closed by this change.

The local/future remote retirement contract is in [TLB_RETIREMENT.md](../TLB_RETIREMENT.md).
