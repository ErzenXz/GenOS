# Logical timer clock on the BSP

`kernel::tick_clock::Clock` supplies monotonic logical ticks to the runtime,
network deadlines and serial input decoder. Before the first hardware timer
delivery, each 4096 coordinator polls advances the logical count once. The
fallback phase always stays below 4096. The first hardware tick advances the
existing count rather than replacing it with a separate counter starting at
one. Every later hardware tick advances that same count. Logical increments
saturate at `u64::MAX` and never wrap.

Hardware delivery permanently disables fallback polling for that boot. If the
timer later stops, polling does not restart time. Poll counts have no physical
elapsed-time guarantee, and this transition does not calibrate fallback polls
against the nominal 100 Hz PIT. Saturation also ends further representable
progress. The clock supplies an ordering and progress policy under those
limits; timer accuracy, missed periods and physical hardware qualification
remain separate work.

## Ownership and interrupt boundary

One permanent `ScopedCell<Clock>` in `interrupts.rs` owns the logical count,
fallback phase and hardware-seen flag. The private `with_clock` adapter masks
ordinary IRQs before attempting a non-waiting exclusive borrow. Every read,
coordinator poll and timer update uses that adapter. Higher-ranked callbacks
return owned values and cannot retain a clock reference. The borrow ends before
the outer scope restores the caller's IF. Enabled callers return enabled;
disabled callers stay disabled, including nested interrupt scopes.

The callbacks do bounded constant work. They cannot enable IRQs, dispatch,
block or reenter the clock. A conflicting borrow is refused before accessing
the payload and halts with IF clear after
`TICK_CLOCK_ACCESS_REJECTED policy=halt`. There is no spin on an interrupted
owner, rollback, reset or recoverable failure path. The adapter introduces no
new unsafe block or unsafe Send/Sync implementation; it relies on the existing
[scoped cell](../kernel/src/scoped_cell.rs) exclusion contract.

IRQ0 retains the raw atomic `TICKS` increment, updates the logical clock and
ends its clock borrow before calling `userspace::timer_preempt`. Existing
preemption, PIC acknowledgement and entry ownership remain responsible for
their own execution boundaries. `hardware_ticks()` exposes the raw hardware
count for observation; scheduling reads `ticks()` or `poll_fallback_tick()`.
The raw counter retains its wrapping atomic semantics and is not a substitute
for logical time. Callers requiring a consistent pair of raw/logical samples
must take both inside an ordinary IRQ-masked scope.

This adapter assumes the admitted BSP and does not establish SMP support or a
general lock order. CLI does not exclude NMI, machine check, synchronous faults
or DMA. Emergency entries cannot call the clock's Rust adapter. Existing
[terminal entry ownership](FAULT_CONTAINMENT.md) supplies their containment.
Measured worst masked duration and complete F4/F5 qualification remain open.

## Verification scope

```sh
cargo test -p kernel --lib tick_clock
python3 -m unittest discover -s tools -p test_tick_clock_harness.py
python3 tools/test_tick_clock.py
```

The pure tests exercise hardware-disabled fallback progress, every fallback
phase at first hardware delivery, updates before/after polling boundaries,
hardware delivery from boot, polling cessation and saturation. They do not
execute CLI, PIT, interrupt entry or hardware delivery.

The immutable-source VM harness adds a bounded probe in main before initial
interrupt enablement. It accrues three fallback ticks with IF clear, enables
the production PIT path, and observes real IRQ progress through the unchanged
clock APIs. It samples raw and logical counts together with IRQs masked and
requires monotonic time, continued hardware increments, no fallback increments
after delivery, and enabled/disabled caller IF preservation. The fixture halts
after its final record. Production clock mutation, timer handler, PIC EOI and
entry assembly remain intact.

For a retained historical source, `--source-ref COMMIT` selects its archive;
the harness adds only the raw diagnostic getter if that source predates it.
This permits a counterexample against the old independent-counter selection
without changing that source's time policy. A failing historical run stays a
failure and cannot substitute for corrected-source qualification.

Build/serial/QEMU logs, fixture diff, QMP transcript, source/archive/image
identity and hashes remain under `build/tick-clock-evidence/`. Harness admission
rejects missing, duplicated, malformed, reordered or resumed records, backward
or frozen time, wrong IRQ/IF progress, CPU faults and reset/stop/exit substitutes.
These commands and checks define the intended evidence; executed commits and
results belong in [VERIFICATION.md](VERIFICATION.md).

## Compatibility and rollback

The public ABI, saved frames, PIT setup and frame/device capacities are unchanged.
The scoped clock replaces two fallback atomics with small permanent metadata;
it allocates no frames or dynamic storage.
Logical time after fallback now retains its accrued offset when hardware begins,
so callers must use differences or deadlines in the same boot's clock domain.
There is no transfer of clock state or live deadlines across a reboot. Reverting
requires a matched kernel/harness bundle and restores the old possibility of
backward time at first delivery. Retest runtime/network deadlines, serial input,
Ring 3 preemption and caller IF before attributing evidence to a changed bundle.
