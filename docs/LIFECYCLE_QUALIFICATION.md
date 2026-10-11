# Single-boot process lifecycle qualification

Run from clean committed source with the normal Rust 1.97.0, QEMU and firmware
dependencies:

```sh
python3 tools/test_lifecycle_churn.py
```

The default is 10,000 process cycles in one validation boot. Odd cycles execute
INIT.ELF in Ring 3 to normal exit; even cycles execute its held mode until real
user output, then kill with status 137. Every cycle reaps the process through the
production manager. A persistent manager reuses its slot with a strictly newer
incarnation; enough cycles force reuse of the bounded display PID range.
Construction, user context switching, syscall dispatch, frame scrubbing, handle
cleanup and reaping remain the actual production paths.

The fixture only adds a call after the existing 257-launch stale-handle probe
and appends a qualification function. All original production functions remain
byte-identical in the archived source. Each cycle requires observed user output,
its exact terminal state/status, successful reap, refusal of a second reap/kill,
empty process slots, zero active processes, the original live-frame baseline,
exact cumulative reclaimed frames, allocator/ledger consistency and restored IF.
The independent host parser additionally matches ordered production image-load,
spawn, output, exit/kill, reclamation and reap records for every cycle. A final
summary alone cannot establish completion. Timer preemption may vary; execution
output and terminal cleanup are mandatory and hardware ticks must advance.

The VM execution budget is 1800 seconds, with a separate 600-second build budget.
A shorter custom deadline may fail but cannot silently extend either limit. The
VM starts paused, is explicitly resumed over retained QMP, and must still report
running after its deliberate final halt. Reset, exit, fault, missing/duplicated/
forged records, changed baseline, incomplete cycles and continuation fail the
run. Raw serial/QEMU/QMP transcripts, fixture/source/archive/image identities,
actual environment, deadlines, results and file hashes remain under
`build/lifecycle-churn-evidence/<run-id>/`. A failed run remains failed.

Replay retained evidence with source/archive, fixture, transcript, QMP and hash checks:

```sh
python3 tools/audit_lifecycle_churn.py build/lifecycle-churn-evidence/RUN_ID
```

The replay reads the fixture literal from the recorded Git commit without
executing archived Python. Source and trace consistency are not a signed
attestation or independent reviewer approval. Failed manifests remain rejected.

For a shorter exploratory run:

```sh
python3 tools/test_lifecycle_churn.py --cycles 20 --timeout 120
```

Use `--cycles 20` with the replay command for the corresponding short run.
Only 10,000 completed cycles set `qualifies_10000_cycles=true`; short passes are
explicitly marked false. The weekly/manual long-validation workflow runs the
fixed 10,000-cycle command and retains failure evidence. A separate required
512-cycle pull-request lane crosses the former byte-counter failure and PID
reuse boundaries; it is regression evidence, not the full lifecycle gate.

## Diagnostic overflow found by this workload

The first full run on `19f53ad248e5ff9f10fc967b794250e16f71ba1a` completed 231
cycles, then panicked at cycle 232 in a valid Ring 3 ping syscall. The production
8-bit atomic returned 255 and ordinary `+ 1` overflowed in the debug validation
image. Release arithmetic would wrap and could repeat a boot milestone. The
original trace and source line 6503 remain at
`build/lifecycle-churn-evidence/1791678519435789000/`; its audit at
`build/lifecycle-counterexample-audit.json` verifies the hashes and rejects the
incomplete run. No fault was injected.

`kernel::diagnostic_counter::DiagnosticCounter` now supplies monotonically
increasing 64-bit totals, saturating at u64::MAX without atomic or returned-value
wrap. Ping, ABI, report and output use it in ordinary syscall dispatch. Exact
initial boot counts remain required; saturation at a boot threshold would hide
extra calls and is deliberately not used. Pure tests cross the byte boundary,
cover overflow at u64::MAX and concurrent event linearization. No unsafe code or
allocation/handle authority is introduced; generation and bounded active-process
counters retain their separate contracts. The selected counters use 32 bytes
instead of four, before surrounding object/linker alignment.

The first corrected-kernel 10,000-cycle run and its 512-cycle counterpart
reached their final summaries, then failed host admission because the initial
parser incorrectly excluded PIDs 1–3. Production admits all nonzero byte PIDs.
The corrected harness checks the complete 1–255 succession and requires reuse
only after 255 cycles. Both original failed manifests stay retained; fresh
qualification runs use the corrected harness. This changes no CPU validator.

## Limits

This workload serially executes one child at a time. Reclaimed-frame totals are
cumulative page lifetimes, not simultaneous memory pressure or proof of physical
remanence erasure. The image is a debug validation kernel with optimized userspace;
this does not establish sustained release-image behavior. Persistent media,
packet/device reset, sustained IPC/file I/O, multiple concurrent children, every
cancellation schedule and arbitrary interrupts remain separate coverage. It
cannot substitute for F6.6's proposed 24-hour mixed pressure or C5's later
72-hour complete console workload. Current local QEMU 11.1.2 is portability
execution against frozen 11.1.1; no reference or release level advances.

Exact corrected-run identities/results belong in [VERIFICATION.md](VERIFICATION.md).
