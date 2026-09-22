# Validation evidence contracts

The local validation, storage, memory, SDK, serial-input and network commands use
`tools/xtask/src/evidence_contract.rs` and `validation_run.rs`. A progress string
inside another message is not success evidence. These harnesses validate the
production kernel and real Ring 3 shell; they do not replace guest behavior with
a host simulation.

Each run must satisfy all of the following:

- Exact validation boot policy and ordered memory-map, CPU-state, page-protection,
  stack-guard, IDT, physical-alias and interrupt-admission records.
- Every required one-shot proof exactly once, with its complete fields and
  declared boot phase. The full validation path also checks the sequential
  subsystem proofs. Network chains enforce device/DHCP, UDP operation/timeout,
  passive accept/stream/concurrency, and fault/budget relationships.
- Complete typed fields on required repeatable diagnostic events. Lifecycle and
  packet events legitimately repeat; counting them as unique would reject valid
  operation. Their repetition never substitutes for a missing aggregate proof.
- Seven unique stack-usage records between console-transcript completion and
  kernel readiness, with the configured capacities, aligned counters, at least
  one page of remaining space and observed boot/IRQ/privilege stack use.
- Network regression counters within the stated retransmission, buffering,
  congestion, completion and throughput budgets. The host independently verifies
  the two HTTP requests and the complete concurrent passive-stream responses,
  half-close and EOF. The no-HTTP variant rejects HTTP/TCP/passive success proofs.
- A fresh run-specific `echo GENOS_RUN_<id>` command sent only after the proof
  contract completes, followed by its exact command echo and response. A stale,
  unsolicited, duplicated or embedded token fails. Validation builds must provide
  both exact `USER_CONSOLE_WRITE pid=... text=...` diagnostics from one process
  and their visible Ring 3 `/> ...` prompt/response; diagnostics alone never pass.
  Normal mode retains its separate untraced `genos> ...` contract. The serial-input lane also
  requires a newly issued `uname`, its command echo, exact ABI/version response
  and receive record; an old boot banner cannot satisfy it.
- The emulator remains alive while collecting another 500 ms of output after
  the challenge response. Failure or duplicate records in that interval fail.
  An early emulator exit is failure even if earlier output looked successful.

A serial log is diagnostic evidence from the tested kernel, not a cryptographic
attestation against a malicious emulator or deliberately forged kernel. The
fresh challenge rejects accidental replay and stale artifacts; source/image
identities and independent reproduction remain necessary release controls.

## Retention and failure paths

Every invocation creates a new directory under
`build/validation-evidence/<run-id>/<label>/` before preflight or emulator launch.
It retains `manifest.txt`, `serial.log` and `qemu.log`. The manifest records the
source commit and working-tree status, environment/profile identity, initial
image/data hashes, actual command, configured liveness deadline, observed boot
readiness/completion time, serial/stderr hashes and failure reason. These timings
are observations, not performance qualification.

A manifest starts incomplete. Errors, early exits and abandoned runs become
failed; an empty transcript cannot be promoted. Child teardown runs on every
return path. Existing flat serial paths are compatibility copies and are never
read to admit the current run. Normal interactive acceptance retains its
separate `build/normal-evidence/` transcript and challenge contract.

The full smoke retains its 12-second minimum after kernel readiness. Validation
storage/memory/SDK phases retain their 60-second deadline, the full network run
120 seconds, and serial/no-HTTP lanes 30 seconds. All include firmware startup.
No deadline was relaxed by the evidence-parser change. A timeout is a failed
liveness bound, not a measurement of expected boot speed.

## Host verification and limits

`cargo test -p xtask` mutates each required record: omission, duplicate one-shot
proofs, embedded text, malformed fields and wrong phases cannot pass. It also
checks numeric budget violations, stack counters, current/stale serial tokens
and failure retention. Small child-process fixtures exercise the actual pipe
reader, command write, post-success observation, early exit and teardown paths.
They require Python 3 but do not build or boot a VM. Compact transcript fixtures
are parser inputs derived from record shapes and extended for the current
contract; they are not acceptance evidence for a guest run.

`cargo clippy -p xtask --all-targets -- -D warnings` checks the harness itself.
After a harness change, run the complete committed-source VM suite and retain its
new evidence before claiming guest acceptance. Host parser tests alone do not
qualify device behavior, storage durability, startup dependencies or long-run
stability. External DNS and router responses in the current network suite are
still external dependencies; the ordinary no-NIC console has a separate test.
