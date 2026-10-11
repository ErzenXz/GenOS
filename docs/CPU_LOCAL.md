# Scoped current-process ownership on the BSP

The current-process binding uses `kernel::context_slot::ContextSlot<UserProcess>`.
The slot holds a raw publication pointer and an atomic borrow flag; it does not
return a global mutable reference. It is one part of F4.1/F4.2/F5.3. It is not an
SMP lock or proof that all CPU-local/global objects have been audited.

## Publication and access

`run_slice` saves IF, disables ordinary interrupts, validates the return context
and copies the address-space value. It derives one process pointer from its
exclusive owner borrow, then suspends use of that owner reference. The slot
publishes the pointer only around CR3 activation, the user trampoline and kernel
root restoration. Its private guard clears publication before prior IF is
restored. Nested dispatch cannot replace a live binding.

The process remains stationary/live in the caller's storage throughout that
scope. Assembly xstate/selector pointers derive from that same publication
pointer, without reborrowing the original owner reference. Assembly accesses
those fields before and after Rust entry callbacks, never during a Rust mutable
borrow. The GPR/return frame resides separately on the IRQ/privilege TSS stack.

Timer preemption, process-local fault termination and syscall entry call the
private `with_current_process` adapter. It unconditionally requires IF clear,
then asks the slot for a higher-ranked callback borrow. A second access is
refused before dereference and reports `CPU_LOCAL_ACCESS_REJECTED policy=halt`.
The private guard releases the borrow before the adapter returns to assembly.
Callbacks return owned values, may not enable interrupts or dispatch another
context, and cannot return a reference tied to the temporary process borrow.
Syscall action handling is separate from that entry adapter.

An empty slot gives timer/fault callers `false` and retains the existing missing
process syscall result. A duplicate publication is a fatal kernel invariant
failure. Borrow rejection is a software ownership denial; it does not pretend
to be a CPU exception. Host unwinding releases both guards. Kernel panic/fatal
entry halts and provides no recoverable unwinding promise.

## Mutable-object classification and unsafe obligations

| Object | Owner and lifetime | Exclusion and retirement |
| --- | --- | --- |
| `INIT_IMAGE` / `SHELL_IMAGE` | Boot publisher, then immutable boot-lifetime bytes | One atomic publication; no replacement, mutable borrow or integer pointer reconstruction |
| `CPU_PAGING` | One record for the admitted BSP | Permanent supervisor root, irreversible bootstrap/sealed phase; active-root copies published after CR3 readback with IF clear |
| PS/2 `STATE` / `ACCESS` | IRQ-shared on the admitted BSP | One masked, non-escaping scope for decoders, queue, bounds and all observations; borrow/publication end before IF restores |
| Memory `STATE` / `ACCESS` | BSP memory coordinator; boot then ready | One masked, reentry-checked scope for bitmap, ledger, fault counter and snapshots; initialization once with IF clear |
| VirtIO DMA allocation / claim | Permanent static bytes; one driver CPU owner | Irreversible static claim before discovery; no drop/failure/reset-based reclamation; token is neither Clone nor Sync; see [DMA obligations](PERMANENT_DMA.md) |
| `CURRENT_PROCESS` binding | One admitted BSP; per-slice publication | Owner reference suspended; kernel root restored before publication clears; IF restored last |
| Slot access flag | One synchronous entry callback | Set before dereference; nested access denied; private scope guard releases before assembly resumes |
| `UserProcess` fields | Coordinator outside a slice; scoped entry borrow during a slice | No owner access while published; reclaim occurs only after leaving the slice |
| xstate/selector field pointers | Trampoline for the current slice | Same process pointer provenance; raw assembly access outside Rust borrow scopes; cleared before trampoline exit |
| GPR/return frame | IRQ/syscall assembly on a guarded TSS stack | Disjoint from process storage; Rust borrow ends before restore/nonlocal exit |

Both public slot operations are unsafe: adapters must supply one owning CPU,
stationary/live storage, suspended owner access, and IRQ/emergency exclusion.
Atomics do not establish those conditions or make cross-CPU access safe. There
is no new unsafe `Send`/`Sync` implementation. The BSP admission gate and assembly
nested-entry guard remain required. Emergency/NMI/#MC paths cannot enter these
Rust callbacks. The complete unsafe inventory still needs its wider caller audit.

Before SMP, each admitted CPU needs its own process slot and paging record plus scheduler,
interrupt-local state and independently reviewed synchronization. The current
slot is a reusable boundary for that work, not a second-CPU implementation.
Other coordinator/device globals, assembly return pointers,
lock order and masked-duration qualification remain open. See
[SINGLE_CORE.md](SINGLE_CORE.md), [FAULT_CONTAINMENT.md](FAULT_CONTAINMENT.md) and
[KNOWN_LIMITATIONS.md](KNOWN_LIMITATIONS.md).

## Verification

Seven host tests cover empty access, owned results/mutation, duplicate publication,
reentrant borrow denial, unwinding/reuse, repeated contexts and assembly field
pointer provenance across successive Rust entries. A compile-fail doctest rejects
returning the mutable reference. The seven tests are included in the pinned Miri
scope with strict provenance; hardware/assembly remains excluded.

`python3 tools/test_fault_containment.py --case syscall-borrow` builds an immutable
fixture that requests a second borrow inside real Ring 3 syscall dispatch. It
changes only the deliberate call/window, leaving the production slot, adapter and
entry guard intact. The evidence requires one ordered window and exact denial,
no second callback, CPU exception, continued boot, reset or stopped VM. The new
independent CI lane is required by the versioned branch-protection contract.
Exact executed commits/results are in [VERIFICATION.md](VERIFICATION.md).

## Immutable boot images

`kernel::boot_image::ImageSlot` replaces both raw image address/length pairs.
Only immutable `&'static [u8]` inputs can be published. A single atomic claimant
writes the length, then releases the original pointer; an acquiring reader sees
both fields from the same slice. The claimant never resets, so neither replacement
nor a competing publisher can alter a published image. Readers never wait: an
incomplete publication remains unavailable. Pointer provenance is preserved through
`AtomicPtr`, with one documented shared-slice reconstruction and no integer casts
or new unsafe `Send`/`Sync` implementation.

Both INIT and shell registration validate the complete native plan before
publication and reject a second registration. Boot rejects an invalid or already
registered shell before launching processes. Each launch still validates admission
and retains its existing mapping/cleanup checks. The interface does not grant
executable authority or expand the accepted image layout. Original initrd storage
must remain immutable and live throughout the boot, as required by the loader's
handoff; this is not a DMA/IOMMU guarantee.

Five host tests cover absence, original slice identity, replacement denial, empty
publication and competing publishers with different slice lengths. A compile-fail
doctest rejects temporary input. These tests are selected for strict-provenance
Miri; the concurrent test exercises its recorded schedule, not every possible
interleaving. This narrows F4.1/F4.2 without closing the wider mutable-state audit.

## CPU-local address-space bookkeeping

`kernel::cpu_paging::CpuPaging` replaces the kernel-root, active-root and sealed-map
raw globals. It contains atomic integer metadata and returns owned root copies;
there are no raw mutable references, unsafe operations or new Send/Sync impls.
A root value from this object grants no allocation, page-table or mapping authority.
The paging adapter still checks the private live root grant before activation.

The permanent root is published once after its exact CR3 readback. During
bootstrap only that root can be active, and permission/guard APIs require the
bootstrap phase. Sealing requires the permanent root active and its hardware
observation matching. The phase never reopens; private-root creation requires
sealed supervisor mappings. Invalid, repeated or mismatched operations preserve
the previous metadata. Readers do not wait for an interrupted initial publisher.

The admitted BSP masks local interrupts across admission, MOV CR3, readback and
active-root publication. A failed post-switch observation reports
`CPU_LOCAL_ROOT_REJECTED reason=hardware-mismatch policy=halt` and halts with IF
clear. It cannot safely guess a root, dispatch, restore interrupts, reclaim frames
or promise hardware rollback. Initialization/sealing mismatch also halts before
runtime admission. Existing local TLB-retirement and ownership checks remain
necessary; atomics do not implement SMP synchronization or shootdown.

Seven host tests exercise phase admission, malformed/unobserved roots, permanent
root replacement denial, bootstrap/private separation, irreversible sealing,
repeated switches, mismatch preservation and competing initial publishers.
They are selected for pinned Miri; simulated observations are metadata-policy
evidence, not privileged CPU execution or exhaustive interleaving exploration.
The full memory translation probe separately performs actual CR3 switches and
same-address reads across roots/reuse, followed by reclamation.

`python3 tools/test_cpu_paging.py` archives clean source and changes only the
observation supplied after a real distinct private-root activation. The production
MMU operation, metadata validator, reporter and entry guards remain intact. It
requires matching actual/target CR3, the declared corrupted sample, the previous
root and one exact ordered refusal; any fault, continued dispatch, reset, exit,
paused VM, duplicate or malformed proof fails. QMP is subscribed before starting
the VM and records final running status after refusal. Success/failure hashes,
serial, command and QMP wire evidence remain in `build/cpu-paging-evidence/`.
This fixture proves software refusal of a falsified sample, not an actual MMU
failure or complete hardware qualification. Scheduler/interrupt-local state and
the wider F4.2/F5 audit remain open.
