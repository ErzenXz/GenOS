# GenOS 0.56 integration verification — 2026-09-08

PR #4 (quality gates), PR #6 (normalized exception entry), and PR #7 (GenOS 0.56 integration) are merged to main. This integration preserves the later GenOS 0.50–0.55 network/SDK work and adds CPU page protections and transactional memory construction. It advances the roadmap without claiming a finished or hardened operating system.

## Proven locally

- Pinned Rust 1.97.0 on Apple Silicon macOS, native QEMU 11.1.1 and EDK2.
- 140 Rust host tests and eight Python evidence-parser regressions pass.
- Formatting, documentation links, strict Clippy for every shipped target, and all CI release-target checks pass. The earlier kernel-binary lint failures are resolved.
- All eight original CPU exception cases pass: user/kernel divide error, invalid opcode, general protection and page fault.
- Six additional CPU protection cases pass with exact fault-address checks: user data/stack NX; kernel IDT/text write protection; SMEP execution; SMAP data access. The default CPU reports optional features absent, while these six probes use `-cpu max` and require both enabled.
- Native QEMU storage creation/restore, corruption/repair/read-only recovery, real serial input, concurrent TCP/loss/reordering, IPv6 SLAAC/DAD/echo and external SDK execution pass.
- Allocation injection fails all ten production process-construction allocation points and restores the live-frame baseline. Host tests cover every allocation in a branched supervisor clone, 1,025 reclaimed frames, and 20,000 randomized fragmented allocation/free operations.
- The normal user disk is separate from all corruption, benchmark and rollback fixtures. Validation packaging/boot errors also attempt production-image restoration and report restoration failure explicitly.

## Reproduction and evidence

`make test` runs the host and native system suite, including memory rollback and the six new protection cases. `cargo xtask test-protections` runs only those six cases. The existing exception workflow separately runs the original eight cases. The workflow definitions remain unchanged; expanded protection testing uses the existing build-and-boot entry point and artifact include patterns.

The CPU harness requires committed source and retains unique per-run manifests with commit, compiler/QEMU versions, fixture patch, image hash, command and serial output. Local evidence is under `build/exception-evidence/`; CI also retains `build/serial-protection-*.log` and `build/serial-protection-manifests.log`. System logs include `serial.log`, `serial-network.log`, `serial-sdk.log` and `serial-memory.log`.

The [development benchmark](PERFORMANCE.md) records the earlier GenOS 0.55 baseline (3.886-second median including firmware and validation work). It is not a current-release or comparative-performance claim. Re-measure with `make bench` on the current tree.

## Remaining limits

The original integration did not include a separate release-image boot; the continuation below closes that local evidence gap. CPU protection coverage does not establish physical-frame-wide W^X, missing-NX or mixed optional-feature VM proofs, emergency-stack guards/nesting, XSTATE, or physical hardware support. The allocator manages up to 8 GiB of usable frames across 64 regions; firmware-map truncation, larger metadata, per-owner frame tokens, sensitive-page scrubbing, contiguous allocations and SMP remain future work. Validation probes ran in normal development boot at the original integration; the continuation below separates those policies.

Production TCP/IPv6 sockets and DNS, identity and reviewed TLS, signed packages, general application launch, userspace graphics/accessibility, ACPI/SMP, xHCI/NVMe, suspend/resume and real-machine validation remain roadmap gates. Run the current console OS with `make run`.

The first integration CI run exposed stranded coalesced VirtIO RX completions. A production-driver queue test reproduced it: one IRQ with two used descriptors delivered only the first. RX now drains the announced bounded batch before waiting for another IRQ, preserves correct recovery accounting, and rejects impossible used-index advances. The existing regression budgets are unchanged.

After RX batching removed artificial delays, the native test exposed a second timing issue: the headless bootstrap used loop iterations as timer ticks, allowing a real network wait to expire before a host packet arrived. The bootstrap now uses the hardware/fallback timer while independently bounding work and elapsed time. The same network gate then observed real readiness wakeups and passed without relaxed budgets.

An intermittent Linux CI persistent-write failure did not recur in eight isolated native serial boots or the next instrumented CI run. ATA status tests did expose two concrete protocol errors: BUSY status could be treated as a completed error, and non-busy DRQ status could be treated as command completion. Those cases now wait correctly. Bounded polling and explicit command-error/timeout diagnostics remain; the exact cause of the earlier CI write failure was not established.

GitHub validation for PR #7: [CI](https://github.com/ErzenXz/GenOS/actions/runs/34171042822) and [exception proofs](https://github.com/ErzenXz/GenOS/actions/runs/34171042829) passed on the final reviewed branch head. The prior local main/history are retained in backup branches.


## Foundation continuation — 2026-09-08

The development branch now separates normal and validation startup, rejects
non-BSP/repeated entry, uses a kernel-owned boot stack, extracts endpoint authority
into a host-testable module, retains deterministic parser regressions, and checks
unsafe source context. It also adds a bounded repeat-boot command and independent
CI jobs, with a separate weekly long-validation lane.

Local evidence:

- `cargo xtask test` passed on immutable source `259a9cb`: storage creation,
  restore, torn-generation recovery, corruption/read-only behavior, serial input,
  networking and fault cases, external SDK execution, every process-construction
  allocation failure, all six CPU protection probes, and both normal boot profiles.
- All eight original CPU exception cases also passed on `5ea37bb`: user/kernel
  divide error, invalid opcode, general protection and page fault.
- All five BSP cases passed on `d0e1420`, including one/four-CPU normal boots,
  repeated kernel/table entry, and an explicitly injected non-BSP identity sample.
- Workspace host tests and strict linting pass, including nine newly executable
  endpoint tests and ordered normal-shell evidence parser tests. Python harness
  tests pass; the checked inventory retains 367 lexical unsafe/assembly sites.
- Parser/socket stress passed 100,000 mutations each for seeds 0 and
  `0x47454e4f53`, plus one million for seed 1, after replaying 12 retained corpus
  inputs and their 693 proper prefixes. The zero-slot socket case is a real
  minimized regression, not just a synthetic acceptance fixture.
- Five consecutive optimized interactive boots passed with fresh disposable
  volumes. Every boot checked missing validation fixtures, process
  launch/status/kill/reap, and persistent write/read. This is not 1000-boot evidence.
- `cargo xtask bench` passes using explicit validation policy and restores the
  normal image. Its report remains a validation-boot observation, with no
  cross-OS or normal-startup performance claim.

The new tests found two concrete bugs. Socket and endpoint slot decoding used an
eager subtraction before rejecting zero. The four-CPU firmware stack also exposed
a 228-KiB entry-frame stack probe that faulted before the first Rust statement;
[ADR 0005](adr/0005-owned-kernel-boot-stack.md) records the kernel-owned stack fix.
A separate release link failed because LTO could not combine the explicit kernel
code model with the precompiled core library; release builds now disable LTO.

Evidence remains under `build/`: `continue-*.log`, `bsp-evidence/`, ordered normal
boot manifests, and copied `final-foundation-evidence/` from the immutable system
run. Logs record the exact tested source. The new CI jobs and scheduled 1000-boot
lane have been configured but have not been executed remotely during this session.
The reference build remains experimental; F0-F7 are not collectively complete.

Final local check totals: **162 workspace Rust tests**, **25 Python harness
regressions**, and **2 parser CLI integration tests** passed. Strict Clippy passed
for the ABI, build tool, UEFI bootloader, kernel library, normal and validation
kernel binaries, runtime, init, both shell policies, and standalone parser harness.
The final normal-shell evidence parser additionally requires each command's own
echo before its response. Both normal profiles and a final committed release boot
passed that stronger check; success manifests now contain one unambiguous status.
The source change is `371a71c`; the build image remains an optimized normal image.

A machine-readable local summary is written to
`build/final-foundation-evidence/summary.json`, alongside retained serial logs,
fixture manifests, exception outcomes, repeat-boot evidence and entry disassembly.
The temporary verification checkout has been removed after copying its evidence.


## Next foundation slice — memory and terminal

Source `e5fefc7` adds zero-before-grant, scrub-before-release, IRQ-scoped allocator
access, stronger consistency checking, and read-only `mem` diagnostics. Source
`e77edd4` adds canonical pathname checks shared with VFS, executable namespace tests,
quiet normal terminal tracing, actual serial clearing and bounded `help` output.
The physical release interface explicitly requires its caller to own and retire
the grant; bitmap membership is not a replacement for owner tokens.

The complete `cargo xtask test` suite passed on `e77edd4`, including storage/network
failure cases, the external SDK, memory poisoning/scrubbing/rollback, nested IF
restoration, all six CPU protections and both normal profiles. Normal tests use
visible echoes/results and require memory usage to rise while a job is running
and return to its pre-launch baseline after teardown. Host checks now comprise
169 Rust tests and 25 Python tests; strict linting and the unsafe inventory pass.
Evidence is retained in `build/next-complete-qemu.log`, `build/serial-memory.log`,
`build/serial-normal-debug.log`, `build/serial-release.log` and their manifests.

The default runner now uses QEMU's serial multiplexer so its Control+A then `x`
exit sequence matches the test-session instructions. `docs/TERMINAL.md` records
what the terminal actually supports. General program launch, shell navigation and
job-control breadth, owner tokens, full concurrency and physical hardware remain
open. This is progress on the foundation, not a production-readiness claim.


The final BSP admission matrix and all eight original user/kernel exception cases
also passed on `a384f90`, along with another 100,000 retained-corpus parser mutations
and two parser CLI tests. This slice's log summary is in
`build/next-foundation-evidence/summary.json`. The generated inventory now records
376 lexical unsafe/assembly sites across 65 source files; that count is retained
review context, not a safety score.

Publishing the series was attempted, but GitHub rejected the branch push because
the connected OAuth app lacks the `workflow` scope required to update
`.github/workflows/ci.yml`. No draft PR was created, and the new CI jobs have not
run remotely. The tested commits remain on `update/normal-boot-hardening` locally.

## September 22 foundation implementation and verification

The complete `cargo xtask test` run passed on **`a3c5349`** using the frozen
`genos-q35-tcg-v3` candidate: Rust 1.97.0, QEMU 11.1.1, recorded EDK2 digest,
versioned machine/CPU, explicit boot devices and dual-stack network settings.
All six boot-memory rejection cases passed on that commit. The later
**`a7ab1d4`** changes only the BSP evidence parser/regression: CPUID's HTT-gated
package-capacity field may be unavailable on the single-vCPU model; it is not
an active-CPU count. All five BSP cases, eight original exception cases,
strict reference acceptance and ten repeated release boots passed on `a7ab1d4`.

Implemented contracts and scoped evidence:

- **F1.1, partial:** checked map decoding and retained boot/kernel/initrd coverage.
  Nine host tests and six actual VM rejection fixtures cover descriptor version,
  count, overlap, unretained initrd/kernel and command-line failure paths.
- **F1.2, partial:** eager private x87/MMX/XMM0–15/MXCSR state, clean construction,
  feature/control-register admission and soft-float kernel policy. A real Ring 3
  fixture passes two reuse rounds, six fresh snapshots, direct/yield syscalls,
  two faults, at least twelve timer preemptions and exact frame reclamation.
- **S1.1 implemented; S1.2/S1.3 partial:** uncertain outcomes, sticky quarantine,
  discarded dirty cache, last-acknowledged RAM reads and transactional mounting.
  Eight host tests include 176 before/after operation failures and 25 torn/recovery
  cases. The corrupt-storage guest proves mutation denial and preserved temporary
  data; it cannot acknowledge volatile `/USER` writes as durable.
- **F0.2/F0.4, partial:** versioned environment identity, explicit boot order,
  isolated firmware writes, fresh serial challenges, ordered/unique CPU evidence,
  retained failure output and independent malformed-handoff CI configuration.
  Remote enforcement and independent reproduction remain absent.

Validation totals: **197 Rust host tests**, **34 Python tests**, **2 parser CLI
tests**, and **100,000 parser mutations** with seed `20260922` passed. The full
QEMU suite includes storage create/restore/read-only/corruption/recovery, serial
input, DHCP/ICMP/DNS/HTTP/concurrent TCP and injected packet failures, the external
SDK application, memory hygiene and all ten construction rollback points, six
page-protection probes, and normal debug/no-NIC and release/network shell flows.
The additional eight exception cases cover user/kernel divide error, undefined
instruction, general protection and page fault. Strict Clippy, formatting,
Markdown links, whitespace and unsafe inventory checks pass. The inventory's
403 lexical sites in 72 files are review context, not a safety score.

Each of the ten final release boots requires real command echoes/results,
persistent file operations, launch/status/kill/reap, a live-frame increase and
return to baseline, and a fresh serial challenge. This does not satisfy the
proposed 1000-boot gate or sustained single-boot soak. No R1/R2/R3 promotion occurs.

The campaign retained and corrected failures: an MMX oracle compared reserved
high bits; duplicated validation helpers exceeded the shell text budget; the
exported SDK omitted its new ABI module; and VM profile assumptions hid firmware
boot selection and disabled IPv4. The corrected VM snapshots boot-image writes
and explicitly enables both network families. Measured loaded-host firmware
startup near twenty seconds justified using the existing sixty-second storage
watchdog; that watchdog is not a performance target. The BSP parser correction
does not relax the one-admitted-CPU requirement. Original failure logs remain local.

Evidence is indexed in `build/roadmap-foundation-evidence/summary.json`. Main logs
under `build/` are `roadmap-foundation-complete.log`, `roadmap-boot-memory.log`,
`roadmap-bsp-verified.log`, `roadmap-exceptions.log`,
`roadmap-reference-qualification.log` and `roadmap-repeat-boots.log`. Per-case
patches, source/image/firmware identities and serial logs remain in
`build/boot-memory-evidence/`, `build/bsp-evidence/`, `build/exception-evidence/`
and `build/normal-evidence/`. These are local results; the new CI configuration
has not run remotely. GenOS test VMs were stopped after verification.
