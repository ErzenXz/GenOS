# Physical frame grants and user mappings

## Decision and scope

A live bitmap bit previously proved only that somebody allocated an address.
The physical release interface now requires a private-field `Grant` carrying an
owner, monotonically increasing allocation generation, ledger slot and frame
kind. `AddressSpace` keeps its root grant and its own never-recycled owner identity.
Owner/grant identity remains a bounded F3 contract. The physical-alias policy
now seals the sole supervisor alias before publishing user read-only/executable
authority; device/shared lifetime rules remain separate.

The pure `frame_grant::Ledger` is the production authority check, exercised by
host tests. `memory` joins it to the existing physical bitmap inside one local
IRQ-masked BSP critical section. NMI/fatal handlers and other CPUs may not use it.
Private fields prevent safe callers from fabricating identities; copying a grant
copies the same authority, not a second frame. Every use revalidates the ledger.
The kernel remains trusted: this is not protection against arbitrary Ring 0 writes.

## Allocation, publication and retirement

1. Reserve ledger capacity and a non-wrapping generation before asking the bitmap
   for a frame. Zero the entire page before publishing its grant. Exhaustion
   returns `None` without a physical allocation or partially published identity.
2. A user page may be mapped only by its address-space owner. The root grant must
   still be live and inactive, the virtual address canonical/in the user range, and the frame
   unbound. W+X remains rejected. Table allocation failures unlink and return only
   the new table grants (inactive roots cannot retain translations from this build); the caller retains the unpublished leaf grant.
3. Publishing one user PTE records its exact virtual address and pins the grant.
   A second alias, a foreign owner, a stale generation or a pinned release fails
   before touching the frame. No general sharing or unpin-by-address interface exists.
4. Teardown refuses the active root and stale root authority. It validates the
   private tree and owner-frame count before mutation; an unpublished outstanding
   leaf is an error, rather than an excuse to release its owner root.
5. On the single admitted CPU, switching away reloads CR3. Bootstrap explicitly
   clears and reads back PCIDE and PGE; user leaves are not global. Every private
   edge is removed before its mapping pin/table grant is retired. Only then can
   release scrub the page and publish the bitmap free bit. A stale address-space
   copy cannot translate, allocate, destroy or activate a reused root.

The TLB choice deliberately avoids retained address-space translations. Intel's
[system-programming manual](https://cdrdv2-public.intel.com/868137/325462-089-sdm-vol-1-2abcd-3abcd-4.pdf),
section 4.10.4.1, specifies CR3 and CR4 invalidation behavior. It is the basis for
this bounded BSP policy, not evidence for SMP shootdown or device translation retirement.

The physical adapter's release and mapping-retirement operations remain unsafe:
callers must retire Rust references, direct-map uses and any device access. The
ledger can reject invalid authority and live user mappings; it cannot observe
arbitrary raw pointers or DMA. Kernel table rollback recovers grants only for the
explicit kernel owner; user-tree walks recover them only under a validated root.

## Bounds and alternatives

The bitmap still represents 8 GiB of usable pages across at most 64 regions.
There are now **8192 simultaneously live grants** (32 MiB of dynamically granted
4-KiB pages, including page tables). Reserved kernel/firmware/static device storage
is outside this count. This explicit live-allocation ceiling is lower than the
managed-address ceiling. Filling it produces bounded allocation failure and an
observable `grant_exhaustion` counter; it never silently discards usable regions.

The ledger occupies at most 328,000 bytes (host size regression enforced), in
addition to the 256-KiB bitmap and region metadata. Lookups and admission scan at
most 8192 records; owner accounting has the same bound. Latency under worst-case
occupancy is not qualified. A dynamically growing allocator or an indexed ledger
requires a separate resource/latency decision before general application heaps.

Keeping raw-address release would preserve the stale-address bug. Dense owner and
generation metadata for every potential 8-GiB page would consume tens of MiB even
when few frames are live. This bounded live ledger trades an explicit capacity
limit and linear lookup for smaller static metadata and testable failure behavior.
The allocation interface grants one order-0 page only, with no multi-call
contiguity promise. Explicit shared grants and device pins remain open. User
owners now have a 64-frame ceiling including page tables; 256 ledger entries
are reserved against user admission. See [MEMORY.md](MEMORY.md) for pressure
and early-boot/runtime contracts.

## Verification and compatibility

Host tests cover wrong owner/kind/address/slot, generation reuse, duplicate release,
mapping pins, aliases, exact retirement, capacity and generation exhaustion,
preparation failure, accounting and metadata size. Denied operations use a callback
that panics if physical preparation is reached.

`cargo xtask test-memory` additionally requires:

```
FRAME_OWNERSHIP_READY stale=denied foreign=denied alias=denied pinned=denied reclaimed=true
```

The probe uses the real mapper/allocator, poisons a page, rejects foreign and
noncanonical mapping without table allocation, rejects a second alias/pinned free,
checks the bytes and both roots, refuses active-root destruction, and proves stale
root/grant denial plus exact reclamation. Existing construction failure cutoffs,
process isolation, exception cleanup, CPU-state reuse and normal launch/kill/reap
exercise this same ownership path. Exact run results belong in VERIFICATION.md.

No userspace ABI, image-layout version or storage format changes. All production
callers migrate together; reverting requires reverting allocator, paging, loaders
and their fixture call sites together. No disk migration is involved. Physical permission changes and supervisor alias retirement are described in
[ADR0007](adr/0007-physical-alias-and-copy-authority.md) and
[TLB_RETIREMENT.md](TLB_RETIREMENT.md). The kernel rejects extra firmware aliases,
freezes bootstrap topology, and denies later user aliases/reprotection. CPU
qualification results remain separate from these implementation contracts.
