# Translation retirement contract

## Current single-CPU implementation

One admitted BSP executes the kernel. Bootstrap clears and reads back CR4.PCIDE
and CR4.PGE before installing the cloned root. Root changes use MOV CR3 without
retention; a stale `AddressSpace` cannot activate a reused frame. IRQ masking
covers the hardware root change and active-root bookkeeping together. This is a
deliberately conservative use of the [Intel SDM's invalidation rules, section
4.10.4.1](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html).

The retirement obligations are:

| Operation | Required ordering |
| --- | --- |
| Bootstrap supervisor restriction/split | Finish all initialized child entries before publishing their parent; invalidate the affected leaf, then reload CR3 before runtime admission |
| Seal a user frame's supervisor alias | Clear alias write authority and INVLPG its address before publishing the user leaf |
| Construct a user mapping | Require a live inactive root; new tables are private, with no retained translations; rollback unlinks new edges before release |
| Switch away | Reload CR3 and update active-root bookkeeping in one IRQ-masked section |
| Destroy a user space | Reject active/stale roots, validate ownership, unlink each entry, restore only its now-retired supervisor alias, retire its exact ledger pin, scrub and free |
| Guard page | Remove its bootstrap identity leaf, INVLPG, and verify absence; reserved backing never enters the free allocator |

Inactive roots cannot acquire new translations on another CPU because no second
CPU is admitted. Their earlier translations disappeared on the CR3 switch away.
Supervisor tables are shared by process roots; changing a managed identity leaf
therefore affects all roots, and INVLPG retires the current root's cached alias.
Other roots cannot retain translations under the disabled PCID/global policy.
There is no runtime shared-kernel topology mutation, user aliasing, huge user page,
user in-place reprotection or kernel alias API after bootstrap sealing.

`TLB_RETIREMENT_READY switched=true reused=true reclaimed=true` requires actual
CPU reads at the same user virtual address across two roots and a replacement
allocation, followed by exact frame reclamation. This complements ownership and
permission-fault probes; a marker alone is not evidence of hardware conformance.

## Required contract before enabling SMP

This is a future implementation obligation, not an implemented shootdown driver.
AP admission remains disabled until the whole protocol is reviewed and tested.

1. A mapping owner must hold the address-space mutation authority, remove/tighten
   PTEs, and publish a nonwrapping retirement generation with the affected range,
   root identity and target CPU set. The set includes CPUs that can retain the
   old root; shared supervisor changes target every online CPU.
2. Every target must observe the PTE writes before invalidating its TLB and
   paging-structure caches with an operation appropriate to its PCID/global-page
   mode. It acknowledges that generation only after local invalidation completes.
   An IPI delivery or queue insertion is not an acknowledgement.
3. Context entry and the target-set snapshot must be synchronized: a CPU joining
   an address space cannot slip between the snapshot and invalidation. A parked
   or offline CPU must invalidate and adopt the current generation before it can
   rejoin or reactivate a retained root.
4. Only after all targets acknowledge may the owner retire mapping references,
   permit writable reuse, scrub a frame, release page tables or recycle identities.
   A delayed/lost acknowledgement keeps the resources pinned. A deadline yields
   a recorded failure/quarantine or deliberate stop; it never authorizes reuse.
5. DMA/IOMMU/device completion requires its separate acknowledgements. CPU TLB
   retirement cannot substitute for device quiescence. Nested IRQ/NMI contexts
   cannot wait for a lock held by the initiating context.

Tests must cover concurrent entry/mutation, delayed/duplicate/stale acknowledgements,
counter exhaustion, CPU offline/rejoin, repeated failure and retained-frame bytes.
Use release/acquire publication plus the architecture's prescribed barriers and
invalidation operations; compiler fences alone do not implement this protocol.
The [kernel TLB documentation](https://cdn.kernel.org/doc/html/latest/core-api/cachetlb.html)
provides a useful independent statement of the visibility obligation after mapping
changes. GenOS must prove its own implementation when SMP is introduced.
