# ADR-0015: Preserve logical time across first hardware delivery

- **Status:** Proposed
- **Date:** 2026-10-13
- **Decision owners:** architecture/runtime maintainers
- **Related roadmap:** F4.2, F5.1, F5.3, F5.4
- **Supersedes:** None; extends ADR-0010's fallback timer boundary
- **Superseded by:** None

## Context and decision

The old timer API selected an independently accumulated fallback counter until
the first hardware tick, then returned the hardware counter. Three observed
fallback ticks could therefore be followed by hardware tick one. Runtime
budgets, serial escape expiry and network deadlines use this shared time domain
and require it to preserve ordering across delayed IRQ delivery.

Keep one pure `Clock` containing a saturating logical count, a fallback phase
bounded below 4096 and an irreversible hardware-seen flag. Before hardware
delivery, every 4096 polls advances logical time. The first and each later
hardware tick advance the existing count; they never reset its origin. Once
hardware delivery begins, coordinator polling cannot advance time. All logical
increments saturate at `u64::MAX`.

Own this object in one permanent `ScopedCell` on the admitted BSP. Its private
adapter masks ordinary IRQs, attempts exclusive access without waiting and
retires the borrow before restoring caller IF. Timer entry releases the clock
before process preemption. Keep the existing raw hardware atomic counter and a
read-only diagnostic getter separate from scheduling time.

## Invariants and failure

Every observation and mutation of the logical object uses the same adapter.
Private callbacks perform bounded constant work, return owned values and cannot
enable IRQs, dispatch, reenter or retain references. A conflicting borrow halts
with IF clear and `TICK_CLOCK_ACCESS_REJECTED policy=halt` before payload access.
It never waits for an owner that an IRQ may have interrupted. There is no
recoverable reset or rollback of clock ownership.

The model requires one admitted BSP. IRQ masking is not an SMP lock and cannot
exclude NMI, machine check or synchronous exceptions. Emergency entries must
remain outside the Rust clock path. Raw hardware observations retain wrapping
atomic semantics; the logical clock alone supplies saturating scheduling time.
Callers needing consistent raw/logical pairs mask IRQs around both samples.

## Alternatives and limits

Selecting the maximum of the old counters prevents a backward sample but freezes
time until hardware catches up with the fallback total. Restarting at the first
IRQ preserves raw hardware numbering while invalidating existing deadlines.
Continuing fallback polling alongside hardware would make progress depend on
coordinator workload after hardware delivery. An IRQ-masked scoped object makes
the transition explicit without a retry loop in interrupt context.

Fallback polls do not measure physical elapsed time. This decision supplies no
calibration, lost-tick recovery or clock restart after hardware stops. The
logical count cannot progress beyond saturation. A new timer source, SMP clock,
physical accuracy claim or recovery policy needs its own qualification. Measured
IRQ latency and broader F4/F5 ownership review remain open.

## Compatibility and rollback

The native ABI, entry frame, PIT programming, preemption interface and
frame/device capacities remain unchanged. The scoped object replaces the two
fallback atomics with small permanent metadata and allocates no frames or
dynamic storage. Hardware time retains any earlier logical fallback
offset, so consumers compare deadlines and differences within one boot. Reboot
creates a fresh time domain; no live deadline or state migration is offered.

Roll back a matched kernel and harness bundle. The earlier counter-selection
regression resumes after rollback and must remain visible in its qualification
record. Revalidate normal runtime/network deadlines, serial input, process
preemption and caller IF whenever the implementation or time source changes.

## Verification and implementation

The implementation seams are `kernel/src/tick_clock.rs`,
`kernel/src/interrupts.rs` and the library export. Pure tests cover fallback
phases, first/later delivery, polling interleavings, disabled hardware and
saturation. Run `cargo test -p kernel --lib tick_clock`.

Run `python3 -m unittest discover -s tools -p test_tick_clock_harness.py` and
`python3 tools/test_tick_clock.py` for harness admission and actual PIT evidence.
The VM fixture preserves production clock/IRQ mutation and observes monotonic
transition, continued hardware progress, polling cessation and caller IF. The
historical `--source-ref` lane can retain the old regression independently.
Source/archive/image identity, fixture and raw QMP/serial evidence are required;
configured commands alone do not establish execution or acceptance.

This ADR stays Proposed pending review and exact-commit evidence in
[VERIFICATION.md](../VERIFICATION.md). It does not close a broader roadmap gate
or promote the release.

## References

- [Logical clock contract and evidence scope](../TICK_CLOCK.md)
- [Single-BSP scheduling and interrupt work](0010-single-bsp-scheduling-and-interrupt-work.md)
- [Interrupt startup admission](../INTERRUPT_ADMISSION.md)
- [Terminal entry ownership](0014-terminal-entry-ownership.md)
