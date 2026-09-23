# ADR-0010: Schedule on one admitted BSP and keep IRQ work bounded

- **Status:** Proposed; codifies the current single-CPU reference policy.
- **Date:** 2026-09-23
- **Decision owners:** kernel/runtime maintainers
- **Related roadmap:** F4.5, F5.1–F5.5, H-SMP
- **Supersedes:** None

## Decision

Exactly one hardware-designated BSP may initialize or run the kernel. It
claims entry before touching shared UART, tables or allocator state and
switches from the firmware stack to an owned guarded stack. The reference
uses a 100 Hz PIT tick and a bounded five-tick task quantum. Timer entry may
preempt an armed Ring 3 process; kernel coordinator work is not a second
preemptible kernel task. Context changes install the exact current process
and CR3 while local maskable IRQs are disabled, then switch back to the kernel
root and clear current-process state before restoring the previous IF bit.

An interrupt handler acknowledges its source and records minimal atomic
readiness. Device descriptor validation, copying, protocol mutation and
blocking waits belong to the runtime coordinator. Nested local IRQ scopes
must preserve the incoming IF state. The normalized vector/error frame and
fatal-versus-process-local policy are in [ADR-0002](0002-normalized-exception-entry.md);
the specific CPU state saved across preemption is in
[CPU_STATE.md](../CPU_STATE.md). Local masking does **not** protect against
NMI, machine check or another CPU and is not an SMP lock.

## Why this trade-off

Starting APs now would make existing raw current-process, scheduler, paging
and driver state concurrently mutable without a reviewed ownership/lock
contract. Permanent polling would avoid some IRQ interaction but delay input
and device completion and would not establish an interrupt-safe design.
One admitted BSP plus short handlers permits deterministic ownership and
fault evidence while the required per-CPU and lock-order work remains open.

## Failure, upgrade and rollback

An unsupported/non-BSP/repeated entrant halts before shared initialization.
An unexpected kernel or machine-level fault deliberately halts; a safe user
fault terminates only that process. A failed optional NIC/storage device
cannot prevent local terminal input. A device IRQ must not wait for a lock
held by the interrupted context. The current source has scoped IRQ rules and
readiness handling, but a complete lock hierarchy, measured worst masked
duration, delayed/nested IRQ stress and same-IST fatal nesting are still F5/F1
gates. The fallback tick before the PIT is active is a boot aid, not a second
qualified timer profile.

Changing machine/CPU count, interrupt controller, tick source or preemption
model creates a new [reference profile](../REFERENCE_VM.md), repeat CPU-state,
fault, latency and ownership tests, and update affected ABI timing claims.
Rollback selects the old complete boot/runtime bundle and its profile; no
live run queue, interrupt epoch or process context is transferable.

## Evidence

The five-case [BSP matrix](../SINGLE_CORE.md), original exception cases,
CPU-state Ring 3 preemption fixture, nested-IF probe, normal reference boots
and [kernel stack tests](../KERNEL_STACKS.md) support this scoped design.
`kernel/src/tasks.rs`, `kernel/src/interrupts.rs`, `kernel/src/runtime.rs`
and `kernel/src/userspace.rs` are the implementation seams. Neither a host
model nor a one/four-vCPU boot qualifies SMP or arbitrary physical timers.
