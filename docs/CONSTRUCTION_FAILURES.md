# Construction and teardown failure audit (F3.4)

This audit covers the current bounded BSP kernel, its fixed user ELF, private
address spaces, and current file/process/endpoint handles. A recoverable error
must leave no live frame or authority created by the failed operation. A failed
invariant during cleanup is fatal: returning an error while silently retaining a
frame or handle would make that contract false. This does not qualify future C1
heap or mapping growth; each new constructor must pass the same gate.

| Boundary | Publication and rollback | Failure evidence |
| --- | --- | --- |
| Supervisor table clone | `clone_supervisor` owns only new table pages and releases the partial subtree on allocation failure. Leaf RAM remains borrowed. The source root is not changed. | Host test injects every allocation cutoff and compares the exact source tree and owned set. |
| Huge-page split for one supervisor page | `protect_kernel_page` saves each original parent entry. On a later allocation or leaf-check failure, it restores all parents, reloads CR3, then scrubs/releases each new child in reverse order. Success reloads CR3 when a huge leaf was split, retiring translations for the entire affected range. | Code audit of the two-child maximum and rollback order; normal/validation boots exercise the sealing path. A synthetic second-allocation failure in the hardware splitter is not yet a separate VM fixture. |
| Multi-page kernel image/physical alias sealing | Each page call is transactional, but an earlier successful split in the same sealing pass remains. Every caller halts during bootstrap on a pass failure; this is **not** a recoverable kernel operation. No process root has been published. | Main boot's fatal error branches and strict reference boot. |
| User root and page mapping | A failed root admission releases its unpublished grant. Mapping records each new table edge, unlinks it and frees children in reverse order on failure. A failed new leaf keeps its frame with the constructor until that frame is explicitly freed. Alias permissions change only after all fallible admission checks; pin/publication cannot recoverably fail. | Host grant/clone tests; production VM ownership probes and all ten process-construction allocation cutoffs with exact live-frame baseline. |
| Partial ELF and stack load | `build_process` owns the private root until the full image and stack are mapped. Every error frees an unlinked leaf, if any, then destroys the complete inactive address space. Teardown validates the tree and owner frame count before mutation. A teardown failure is fatal rather than reported as a successful rollback. | Ten allocation cutoffs include partial image/stack states. Invalid ELF/layout checks reject the image before a runnable process is published. |
| Process publication and teardown | Failed pair/fan-in construction reclaims each earlier process. A failed post-build process-handle allocation reclaims the uncommitted child. Normal exit, kill, fault, supervisor cleanup and SDK/init probes reclaim before a slot is reused. Supervisor cleanup now also reclaims already-completed children before discarding their slots. | Transactional rollback, supervisor cleanup, process-generation and lifecycle probes check slot/handle state, active counts and reclaimed-frame counters. |
| File/process handle creation and close | A file handle registers authority before inserting its optional immutable snapshot; snapshot insertion failure unregisters authority or halts if that rollback invariant fails. Capability slots publish last. Close verifies registry removal before clearing a slot; stale and wrong-kind handles leave state unchanged. Process-handle creation similarly publishes its slot last. | Host handle/snapshot tests; VM full-table, stale-handle, close/reuse, copy-out failure and cancellation probes. |
| Socket creation and accept | If unified-handle registration fails after socket allocation/accept, the socket is closed before returning an error. A failed close or a post-close handle-unregister failure halts rather than retaining untracked state. | Network/socket host and VM tests cover open, accept, close, stale handles and capacity. |
| RAM VFS node construction | Oversized create, overwrite and append reject before changing a node or namespace slot. Required boot root/initrd seeds halt on construction failure. | Host oversized-operation test compares original bytes and namespace count. |

The current `Owner` identifier is monotonic and intentionally never recycled,
even after failed construction. Consuming an ID is not a live frame or usable
authority. Frame-grant allocation identities likewise never wrap or recycle.

The audit does **not** claim DMA buffer retirement, SMP teardown, arbitrary ELF
applications, generalized filesystem handles or C1 dynamic mapping/heap growth.
Those have their own later gates. See [FRAME_GRANTS.md](FRAME_GRANTS.md),
[MEMORY.md](MEMORY.md), and [VERIFICATION.md](VERIFICATION.md) for the current
ownership model and test commands.
