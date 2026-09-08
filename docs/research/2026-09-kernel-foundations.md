# Kernel foundations research for the console-first roadmap

Researched 2026-09-08. This note supports roadmap decisions; it does not certify
GenOS or report a new test run. Repository context was read at `5599dc7`, mainly
[the memory contract](../MEMORY.md), [the single-core contract](../SINGLE_CORE.md),
`kernel/src/memory.rs`, and `kernel/src/arch.rs`.

The existing Rust monolithic kernel, capabilities, normalized exception entry,
page protections, frame scrubbing and BSP admission are useful foundations. The
next step is to complete their ownership and failure contracts. Nothing in this
research justifies replacing the architecture simply because another OS has a
different one.

## Primary-source findings

These ten sources were opened and read. Intel's current PDFs were downloaded
from its official download links and their text inspected because browser PDF
retrieval failed. Both identify revision 092, June 2026; section numbers below
refer to that revision. An older 2016 PDF encountered during research is not the
basis for the current section references.

1. **CPU state is an explicit OS responsibility.** XSAVE support is enumerated
   through CPUID; enabling it requires appropriate CR4/XCR0 settings. CPUID leaf
   `0xD`, subleaf zero, distinguishes space required for supported components
   from space required for components currently enabled in XCR0. XSAVE areas
   require 64-byte alignment. Clearing XCR0 bits is not a universal mechanism
   for disabling all associated features: x87 and SSE are notable exceptions.
   [Intel SDM Volume 1, §§13.1–13.4](https://cdrdv2.intel.com/v1/dl/getContent/671436).

2. **Page reuse and exceptions require more than changing flags.** A mapping
   change can leave cached translations; backing frames must not be reused until
   the appropriate invalidations finish. Multiple active CPUs require coordinated
   invalidation. IF does not mask NMIs or processor exceptions. IST supplies a
   known stack by loading a configured stack pointer, so its existence alone
   does not establish safe nesting. With ordinary IDT delivery, IRET unblocks
   NMIs even when it faults.
   [Intel SDM Volume 3A, §§5.10.4–5.10.5, 7.7–7.8, 7.14.5](https://cdrdv2.intel.com/v1/dl/getContent/671190).

3. **Firmware handoff has a versioned, changing memory-map contract.** The map
   key changes when the map changes; `ExitBootServices` needs the current key and
   may require retrying the map/exit sequence. Software must advance descriptors
   by returned `DescriptorSize`, not an assumed struct size. Allocating a larger
   map buffer can itself grow the map. Boot-services memory becomes available
   after successful exit; this does not make every memory type allocatable.
   [UEFI 2.10 Errata A, §§7.2.3 and 7.4.6](https://uefi.org/specs/UEFI/2.10_A/07_Services_Boot_Services.html).

4. **Rust does not excuse unsafe boundary violations.** Undefined behavior
   includes data races, dangling/misaligned accesses, invalid values, incorrect
   assembly and ABI use, and violating aliasing requirements. An unsafe
   abstraction must remain valid for any safe caller allowed by its interface;
   the reference explicitly says its list is not exhaustive.
   [Rust Reference: behavior considered undefined](https://doc.rust-lang.org/reference/behavior-considered-undefined.html).

5. **Volatile is not synchronization.** Volatile access has compiler-observability
   guarantees, but behaves like non-atomic access for concurrency. Within Rust
   allocations the usual access rules still apply. MMIO has a separate contract;
   it should not be treated as ordinary shared RAM.
   [Rust `read_volatile` documentation](https://doc.rust-lang.org/std/ptr/fn.read_volatile.html).

6. **Lock correctness includes interrupt context.** Linux's validator tracks
   lock classes, order and IRQ usage. Taking a lock with interrupts enabled when
   an interrupt handler can take the same lock creates a recursion/deadlock
   hazard. Inverse acquisition order is another rejected dependency. The useful
   lesson is explicit context and dependency tracking, without importing Linux's
   entire locking implementation.
   [Linux locking correctness validator](https://docs.kernel.org/locking/lockdep-design.html).

7. **Miri adds evidence on executions of testable Rust code.** It detects many
   memory and aliasing errors and some race/weak-memory behavior, but cannot
   establish soundness, explore every execution, or run arbitrary platform APIs.
   Its own documentation explicitly describes these limitations.
   [Miri project documentation](https://github.com/rust-lang/miri).

8. **Fuzz counts need coverage context.** Cargo-fuzz's coverage workflow replays
   a corpus against an instrumented program. Its documentation recommends using
   the report to discover unreached paths and improve seeds or targets.
   [Rust Fuzz Book: coverage](https://rust-fuzz.github.io/book/cargo-fuzz/coverage.html).

9. **Small formal checks are feasible, with explicit scope.** Kani checks Rust
   proof harnesses for assertions and selected safety properties. A run can prove
   a property, find a counterexample, or exhaust resources; feature support is
   incomplete, including concurrency. This makes it a candidate for bounded
   sequential ownership models, not an automatic whole-kernel proof.
   [Kani getting started](https://model-checking.github.io/kani/).

10. **Even formal assurance needs a stated boundary.** seL4 describes assumptions
    involving hardware, assembly, boot, memory-management machinery and DMA, and
    limitations concerning timing channels. Borrow the discipline of stating
    assumptions and verified configurations; adopting Rust, capabilities, or a
    microkernel does not transfer seL4's proofs to GenOS.
    [seL4 verification assumptions](https://sel4.systems/Verification/assumptions.html).

## GenOS recommendations and acceptance criteria

The priorities, implementation choices, numbers and gates below are proposed
engineering decisions for GenOS, not requirements imposed by the cited projects.
Each completion claim should name the source commit, toolchain, configuration,
test command and retained evidence.

### K1 — Make frame and mapping ownership enforceable

Introduce a non-forgeable frame grant at the safe interface, with owner identity
and allocation generation where runtime identity is required. Keep physical
addresses as addresses, not proof of ownership. Define explicit shared grants,
mapping references, and pinned device buffers. Decompose release into retirement
of references/mappings/device use, required invalidation, scrubbing and free-list
publication. Forbid executable/writable aliases according to a documented policy
that includes the kernel direct map and temporary loader mappings.

Prerequisites: classify current page users and unsafe release sites; decide which
sharing is actually supported. Tradeoff: metadata and API migration add work and
memory cost; a bounded table is acceptable initially if overflow fails cleanly.
Reference-VM acceptance:

- Foreign-owner, stale-generation, duplicate, misaligned and unissued releases
  leave allocation state and bytes unchanged.
- Every process-construction failure point restores its frame/handle baseline;
  teardown with active aliases or pins is rejected or completes a documented
  retirement protocol before reuse.
- Cross-process read/write/execute attacks and writable aliases of executable
  frames fail in QEMU; tests exercise actual access faults, not only PTE values.
- Repeated map/unmap/protect/reuse cycles cannot observe stale translations or
  another process's previous bytes. Allocation exhaustion returns bounded errors.

### K2 — Complete the CPU context and exception contract before general apps

Specify the supported userspace ISA and preserve every allowed process-visible
state component across preemption, syscall, fault and process lifecycle. Prefer
eager save/restore for the first implementation because ownership is easier to
audit. Select a bounded CPUID-validated XSAVE policy or an explicit FXSAVE fallback
for a narrower supported ISA; do not enable every new state component by default.
Audit kernel compiler features and any kernel use of vector registers. Give all
kernel/privilege/emergency stacks inaccessible guard pages and measurable usage.

Prerequisites: an ABI register inventory, feature admission policy, safe backing
storage and transition ownership. Tradeoff: eager state transfer costs time and
memory; measure that cost before optimizing. Reference-VM acceptance:

- Two adversarial processes retain distinct x87/SSE values and control state
  through many alternating slices; include each additional enabled component.
- A new process receives defined initial state and cannot read a predecessor's
  register state. Disabled ISA use causes the documented process fault.
- Missing XSAVE, unsupported masks and oversized save areas select the documented
  fallback or refuse admission before an unsupported instruction executes.
- Stack overflow reaches a controlled fault path with retained evidence rather
  than corrupting an adjacent object or producing an unexplained reset.
- Nested fault/NMI and return-fault cases have a reviewed policy: recover only
  where correctness is established; otherwise halt predictably on a safe path.

### K3 — Finish single-core ownership; make SMP a separate milestone

Keep BSP-only admission while auditing shared state. Classify each object as
boot-only, coordinator-owned, IRQ-shared, or emergency-safe. Use scoped access
that cannot leak mutable references; preserve incoming IF on nested critical
sections. Interrupt handlers should record bounded work and acknowledge devices;
ordinary processing consumes it. Add debug assertions for context and lock order
as locks appear, including assertions against blocking while IRQs are masked.

Prerequisites: a full mutable-global inventory and the K1/K2 lifecycle boundaries.
Tradeoff: short IRQ-masked regions simplify the first release but impose latency
costs; measure worst observed masked duration under the reference workload.
Reference-VM acceptance:

- Every shared global has an owner, allowed execution contexts and access API;
  every unsafe `Send`/`Sync` implementation has reviewed caller obligations.
- Delayed and nested interrupts during allocation, lifecycle and device work
  preserve counters, queues and forward progress; no handler waits on interrupted
  work that owns its lock.
- Diagnostic readers get a stable per-open snapshot or a versioned retry contract.
- Booting the supported multi-vCPU VM still admits exactly one kernel CPU; this
  is recorded as single-core evidence, never as SMP success.

SMP prerequisites are reviewed AP startup/parking, per-CPU descriptor tables and
stacks, scheduler/address-space ownership, lock hierarchy, memory-ordering rules,
and acknowledged TLB retirement. SMP acceptance must include simultaneous
cross-CPU mapping changes, teardown and delayed acknowledgements before pages
are reused. A host atomic test cannot substitute for that evidence.

### K4 — Treat boot input as a checked handoff

Parse the entire firmware map with checked length/stride/arithmetic and reserve
kernel, boot data, tables, stacks, firmware runtime regions and device mappings.
Do not silently lose descriptors at a static capacity boundary. A supported RAM
ceiling is acceptable if memory above it is explicitly excluded and reported.

Prerequisites: one versioned BootInfo contract shared by producer and consumer.
Tradeoff: rejecting an unsupported map is less convenient than best-effort boot
but makes the supported configuration honest. Acceptance: malformed/overlapping
maps, odd descriptor strides, map growth and stale exit keys are injected; every
case either yields a consistent non-overlapping allocator map or a clear rejection
without out-of-range writes. Preserve fixtures for boundary RAM/region counts.

### K5 — Build layered evidence, not a larger test-count headline

Run host unit/property tests on pure allocator, handle, parsing and state-machine
logic. Add Miri where platform-independent execution is possible. Pilot Kani on
one small ownership/generation model with explicit finite bounds and assumptions;
retain it only if useful. Add coverage-guided parser and operation-sequence fuzz
targets with invariants beyond absence of crashes. Keep actual page faults,
assembly, interrupt delivery and device interactions in QEMU/hardware tests.

Prerequisites: narrow interfaces that separate policy from hardware operations.
Tradeoff: nightly/model-checking tools add maintenance and execution cost; pin
versions and report unsupported operations rather than disabling checks silently.
Acceptance:

- Fuzz artifacts identify target, corpus, seed where available, duration, compiler,
  sanitizer settings and coverage. Crashes become minimized regression fixtures.
- Corrupted lengths, user pointers, allocation failure and cancellation restore
  invariants; liveness tests have deadlines and retain timeout diagnostics.
- Complete the proposed 1,000 fresh-boot gate, then a separate sustained single-
  boot workload mixing process churn, memory pressure, storage and terminal use.
  Require zero unexplained resets, hangs, corruption or resource growth after
  expected caches reach their documented bounds. Report duration and workload.
- Release evidence maps claims to checks and explicitly lists remaining unsafe,
  assembly, firmware, emulator and device assumptions. No tool pass becomes a
  whole-kernel safety or formal-verification claim.

## Boundary between the reference console OS and broader qualification

K1–K5 belong before calling the supported single-core reference-VM kernel
dependable. They do not, alone, deliver a useful terminal OS: applications,
filesystem recovery, streams, cancellation and shell usability need their own
acceptance gates in the main roadmap.

Real-hardware qualification adds a specific supported machine, firmware revisions,
CPU-vendor checks, interrupt routing, timers, DMA/device lifetime and fault policy,
power transitions and recovery tests. Before claiming containment against hostile
DMA devices, establish an IOMMU policy and verify it on supported hardware; a
normal virtio VM run establishes neither that isolation nor physical reliability.
Security against untrusted software additionally needs an explicit threat model,
authority audit and mitigation policy for supported CPUs. These remain separate
from feature completeness and from the graphical UI, which can remain deferred.
