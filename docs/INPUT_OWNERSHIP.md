# PS/2 input ownership on the BSP

The legacy PS/2 path remains compiled while the product console uses serial
input. Its queue, keyboard decoder, partial mouse packet, pointer state and
bounds now form one `kernel::input_state::InputState`. The pure object has private
fields and returns owned events, counts and pointer snapshots. It performs no
hardware I/O or synchronization by itself.

## Hardware adapter

One permanent private `STATE` replaces six independently mutated globals.
`input_hw::with_input` masks local IRQs before publishing its original pointer
through `ContextSlot`. It creates the mutable reference only inside a
higher-ranked callback; the borrow and publication guards both end before the
outer interrupt scope restores the caller's IF. No owner reference or other
access to this object exists. Reentrant publication/access is refused before
dereference and halts with IF clear, rather than waiting on the interrupted
caller. No new unsafe Send/Sync implementation is introduced.

Every coordinator operation uses the same scope, including peek, queue depth and
pointer observations. Keyboard/mouse IRQ functions require IF clear and use the
same adapter. Each IRQ reads at most one ready byte from its own controller
source; the coordinator polling method consumes at most eight ready bytes.
Decoding and queue insertion have fixed storage and bounded work. Queue overflow
keeps the newest 64 events and counts drops; pointer state still updates when
events are dropped. Keyboard modifiers and partial mouse packets remain
independent when bytes interleave. Pointer initialization clamps degenerate
dimensions to a valid position, and position updates saturate before clamping.

The BSP admission gate is essential. Masking IRQs is not an SMP lock and does not
exclude NMI, machine-check, synchronous faults or DMA. Emergency entries cannot
call input Rust code, and this object is not shared with a device through DMA.
Private callbacks must not enable interrupts, dispatch, block or reenter input.
Fatal entry halts rather than promising recoverable unwinding.

PS/2 controller/mouse configuration remains boot-only and requires IF clear.
Its existing bounded 100,000-iteration readiness/ACK waits execute outside any
input-state borrow. Those waits still lack explicit device-error propagation;
this work does not qualify physical PS/2 devices or runtime reconfiguration.
The serial-first boot does not call that optional mouse initialization path.
Worst-case masked time under the complete workload remains an open F5.2 item.

## Verification scope

Six production host tests cover missing-output status, interleaved decoding,
queue overflow/order with pointer progress, motion/button coordinates, degenerate
bounds and integer-boundary movement. They are selected in the pinned
strict-provenance Miri lane. The reused `ContextSlot` tests and compile-fail case
exercise scoped exclusivity and borrow escape; hardware IRQ exclusion needs VM
evidence separately.

`python3 tools/test_input_irq.py` archives clean source and inserts only a bounded
coordinator window and its boot call. It holds the production input borrow with
an intentional bounded fixture delay, extending the normal short scope, and
IF clear while the host sends a real QMP key-down event. A serial handshake
releases the window after QMP acknowledges the input request. Before release the
guest requires a ready keyboard byte, unchanged hardware IRQ count and an empty
queue. After the scope restores IF, the actual production IRQ must deliver
exactly one `a` event. Peek/pop/depth and a nested masked polling scope then check
queue retirement and both enabled/disabled caller IF preservation. The fixture
halts after its final record. It does not replace the decoder, queue, IRQ
handler, adapter, PIC acknowledgement or interrupt entry assembly.

The wrapper retains source/archive and image identities, fixture diff, serial
handshake, raw pre-start QMP commands/events and final running status in
`build/input-irq-evidence/`. Missing, duplicate, reordered or forged phases,
early delivery, wrong key, missing IF restoration, unexpected continuation,
CPU faults, reset/stop/exit or a deadline failure are rejected. This is one
delayed keyboard IRQ schedule, not mouse-device, exhaustive nesting, SMP,
allocator/device-completion or whole F4/F5 qualification. Executed commits and
results belong in [VERIFICATION.md](VERIFICATION.md).
