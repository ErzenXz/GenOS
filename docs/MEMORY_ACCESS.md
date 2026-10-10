# Scoped physical memory access

`memory::MemoryState` keeps the bitmap allocator, grant ledger, initialization
state and validation-only allocation-fault counter in permanent in-place storage.
The bitmap and ledger are not copied onto a kernel stack. Their capacities,
grant identities, quotas, mapping pins and zero/scrub semantics are unchanged.

One admitted BSP owns this object. A private `with_state` adapter masks ordinary
IRQs before publishing its original pointer through `ContextSlot`. Only the
slot's higher-ranked callback creates the mutable reference. No external owner
reference or direct access to the fields exists. Borrow and publication guards
both end before the caller's previous IF is restored. All allocator/ledger
observations and validation fault configuration use this same boundary.

Initialization requires IF clear and may begin once. It validates the firmware
map, admits usable regions and checks bitmap consistency before publishing the
ready phase. Serial diagnostics use copies after leaving the state borrow.
Invalid maps/capacity halt; partial initialization does not become ready, and
there is no retry or recoverable initialization rollback promise. Ready methods
deny access before initialization. Repeated initialization is refused before
another map admission, and initialization with IF enabled is refused even when
the object was already initialized.

Reentry into the memory scope refuses before another reference is created:
`MEMORY_ACCESS_REJECTED policy=halt`. Phase/context refusal also clears IF and
halts; it cannot dispatch or restore the interrupted caller. No handler waits
on a lock held by that caller. Private callbacks cannot enable IRQs, switch
contexts, reenter memory or retain references. The kernel does not unwind after
a fatal entry. No new unsafe Send/Sync implementation is introduced.

Allocation keeps grant/bitmap publication and page zeroing inside one scope,
before returning the grant. Release still checks identity and pins before the
bitmap's scrub callback; callers must retire references, translations, mappings
and device use first. Scoped metadata access does not discharge those unsafe
release obligations. See [local TLB retirement](TLB_RETIREMENT.md).

This is single-BSP exclusion, not an SMP lock. IRQ masking cannot exclude NMI,
machine-check, synchronous faults or DMA; emergency paths cannot enter this
Rust adapter. Ordinary IRQ handlers do not allocate/reclaim memory. Process
entry may call memory with IF already clear; memory never dispatches process
entry or input. Future changes must preserve this ordering and the emergency
boundary. Worst-case masked allocation/scrub/report duration under the complete
workload remains open, as do broader device/reference retirement and F4/F5.

## Verification

`python3 tools/test_memory_access.py` builds five separate immutable fixtures;
`--case` selects one. They retain exact source/archive/image identity, fixture
diff, serial logs, raw pre-start QMP commands/events and final running status in
`build/memory-access-evidence/`. Each case is an independent required CI job:

- `before-init` calls a production observation before initialization and requires
  the uninitialized-phase refusal, without a bitmap-ready record.
- `init-repeat` initializes normally, then requires repeated initialization to
  refuse before any further map admission.
- `init-if-enabled` calls initialization after real IRQ enable and requires the
  context refusal rather than the already-initialized reason.
- `reentrant` requests a production observation inside the allocation's live
  state borrow and requires rejection before nested access/dispatch.
- `irq-deferred` inserts an intentionally bounded delay after a real grant is
  allocated and before production zeroing. QMP sends one real keyboard event;
  a serial handshake releases the window after acknowledgement. The guest
  requires a pending controller byte, unchanged IRQ count and empty input queue
  while memory is borrowed with IF clear. After scope retirement/restoration,
  the actual handler must deliver exactly one `a`. The returned grant must be
  live and zeroed; poisoning/release must scrub it and restore the frame baseline.
  A nested masked scope also verifies disabled/enabled caller IF preservation.

The fixtures preserve the production slot, phase checks, allocator, ledger,
zeroing, scrubbing, IRQ handler, PIC acknowledgement and entry assembly.
Malformed, missing, duplicated, reordered or wrong-reason proof, partial
continuation, CPU faults, reset/stop/exit and deadlines fail. The four refusals
are software denial evidence, and the allocation case covers one delayed
keyboard schedule. They do not establish exhaustive interrupt nesting, physical
device qualification, SMP or complete memory lifetime safety. The reused slot
and pure allocator/ledger tests run under pinned Miri; privileged adapter and
physical memory operations are outside interpreter scope. Executed commits and
results are recorded in [VERIFICATION.md](VERIFICATION.md).
