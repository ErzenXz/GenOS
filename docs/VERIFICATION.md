# GenOS 0.56 verification record

The latest completed campaign covers frame ownership, guarded kernel stacks,
kernel ELF admission and coherent diagnostic handles. See the
[September 22 hardening results](#frame-ownership-and-stack-hardening--2026-09-22)
for current totals and exact source identities. Earlier entries retain their
original scope and dates.

## Original integration — 2026-09-08

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


## Frame ownership and stack hardening — 2026-09-22

The complete `cargo xtask test` suite, six boot-memory rejection cases, five BSP
admission cases, eight original user/kernel exception cases, and strict
`cargo xtask test-reference` passed on **`2df68f5`** in a clean detached checkout.
The four new kernel-stack fault cases and the new Ring 3 memory-snapshot case
also passed on that commit. The strengthened six-case ELF rejection campaign
passed on **`f5b1aac`**, requiring actual QMP guest-shutdown evidence. That later
commit changes only the ELF test harness, its tests and documentation: kernel,
bootloader, ABI, userspace and reference-profile sources are identical to `2df68f5`.

| Delivered slice | Executable evidence | Remaining boundary |
| --- | --- | --- |
| Bounded kernel ELF plan before allocation/copy | 14 host tests; actual debug/release images accepted; six malformed staged images rejected by the real loader, before kernel entry, with explicit guest shutdown | Firmware allocation/map/exit fault campaign, authenticity and other firmware implementations |
| Seven kernel stacks with fourteen guards | Four host geometry tests; all guard mappings checked during boot; four exact CPU fault/CR2/controlled-halt cases | High-water measurements, guard-skipping, same-IST/NMI nesting and return faults |
| Owner/generation frame grants and user mapping pins | Seven host tests; production VM probe rejects foreign, stale, aliased, pinned and active-root operations without erasing owned bytes; all ten allocation rollback points reclaim exactly | Kernel direct-map W^X, sharing, DMA/reference retirement, quotas and SMP |
| Immutable reports per open handle | Five host tests plus real Ring 3 interleaved seven-byte reads, stable stat/EOF, stale-handle denial and two full-capacity close/reuse cycles | Dedicated process-death-with-open-snapshot CPU case and broader pressure policy |

The snapshot VM measured **1043 → 1053 → 1043 live frames**: launching a held
process changed the second report while the first retained its exact original
contents; kill/reap restored the live-frame baseline. The ordinary shell and ABI
remain unchanged. The owned allocator now has a separate 8192-live-grant limit
(32 MiB including page tables), while the bitmap still covers up to 8 GiB of
usable pages in 64 ranges. Its ledger uses at most 328,000 bytes; worst-case
lookup/IRQ latency is not qualified. These bounds are part of the delivered
contract, not a claim of general memory scalability.

Current local totals: **228 Rust host tests**, **58 Python tests**, **2 parser CLI
tests**, and **100,000 parser mutations** with seed `20260923` passed. The parser
run also replayed the 12-input corpus and 693 proper prefixes. Strict Clippy for
the changed kernel/loader/tool modules and all shipped user targets, formatting,
Markdown links, whitespace and unsafe-context checks pass. The updated inventory
records **419 lexical sites in 80 source files**; it is review context, not proof
that all unsafe assumptions are discharged.

The full integration suite covers storage creation/restore/corruption/recovery,
serial input, DHCP/ICMP/DNS/HTTP/concurrent TCP and injected packet faults, external
SDK execution, memory hygiene/ownership/rollback, six CPU page-protection cases,
and normal debug/release shell workflows. The additional CPU/handoff matrices
plus the eleven new guard/ELF/snapshot cases provide **36 scoped case manifests**;
the full suite and strict reference run retain **four normal-boot acceptance
manifests**. This is not a sustained soak or a 1000-boot result.

Review caught an active-root mapping rollback risk: intermediate translations
could remain cached after failed construction. The mapper now requires an
inactive root before any mutation. Review also caught an evidence gap: QEMU
exit zero with `-no-reboot` could mean a reset. ELF tests now subscribe to QMP
before starting the guest, require `guest=true`/`reason=guest-shutdown`, reject
reset/panic/host exits, and bound message sizes, counts, deadlines and the
EOF/process-exit race. All six cases passed with the stronger oracle.

One initial isolated host invocation failed before boot because an inherited
`SDKROOT` selected SDK stubs incompatible with its host linker. Repeating the
campaign with the normal project environment and Python 3.14.7 passed; the
failed log/manifest remain retained. This does not resolve independent host-SDK
reproduction or the other F0.5 requirements.

Evidence is indexed in `build/roadmap-hardening-evidence/summary.json`. The copied
clean-checkout campaign and all its logs/manifests are under its `core/` directory;
new guards, explicit-QMP ELF rejections and diagnostic handles are under
`guards/`, `elf/` and `snapshot/`. The index also records test totals, hashes,
source identities and retained earlier failures/superseded evidence. Original
per-case source paths remain in immutable manifests; fixture archives/checkouts
are temporary and their retained copies supply the lasting local record.

Eleven independent guard/ELF/snapshot CI cases are configured in
`.github/workflows/foundation-hardening.yml`. They have not run remotely; the
previous workflow-scope publication restriction remains unresolved. No release
promotion occurs: GenOS remains Experimental, and the roadmap now names the
remaining alias, sharing/device, CPU/stack and console-platform work explicitly.

## September 23 kernel-and-console checklist: 40 of 100

The [roadmap](../ROADMAP.md) retains the original 100-entry kernel-and-console
checklist from `98c8b38b809f5948edf4d90cc672f22a3013bde4`, when 26 entries
were checked. `python3 tools/console_progress.py` verifies every original criterion
line against `tools/console-progress-baseline.json` and reports **40/100 (40%)**:
33/60 kernel foundations, 5/8 bounded-storage criteria, 2/19 application/terminal
criteria and 0/13 console-qualification criteria. The newly checked IDs are F0.4,
F1.3, F1.5, F2.1, F2.2, F3.3, F3.5, F5.5, F6.3, F7.1, F7.3, S1.2, S1.3 and S1.4. This is a
criterion count, not an estimate of effort, release readiness or safety.

The clean full-suite source was **`34756d1`** (`1026634` contains the final
network-validation race fix). Kernel and bootloader source has not changed since
the independently tested `ff3ab1e` foundation commit. `cargo xtask test` and
`cargo xtask test-reference` both passed on `34756d1`; the strict reference run
matched Rust 1.97.0, QEMU 11.1.1, firmware SHA-256
`33090cc07675baa5190d9f1e84bf5176b33bcbfa9bacac522961150cdb6dbb2a`
and `genos-q35-tcg-v3`. The full run retained ten passing validation VM manifests,
six exact CPU protection fault results, and normal debug/release acceptance;
strict reference repeated the two normal modes with a clean-source guard.
`build/forty-percent-evidence/summary.json` indexes and hashes the fourteen
passing VM manifests and primary logs.

| Scoped evidence | Retained location and result |
| --- | --- |
| Full integration and strict normal reference | `build/forty-percent-full-34756d1.log` and `build/forty-percent-reference-34756d1.log`; four passing normal manifests under `build/normal-evidence/` |
| Stack containment | `build/stack-guard-evidence/1790077416619100000/`; all 16 cases passed, including fourteen exact guards, seven usage readings and a 12-KiB compiler frame crossing into its guard. Validation remaining stack budget exceeded 4 KiB |
| CPU admission and ISA | `build/cpu-feature-evidence/1790078218598487000/`; all 18 kernel-lane cases passed. Actual FXSR/SSE-disabled QEMU variants fault in firmware before GenOS, retained separately as unsupported platform failures in `build/cpu-feature-summary-ff3ab1e-7d6a383.json` |
| Physical aliases, copying, quotas and local TLB | Three hardware alias faults passed in `build/forty-percent-alias-campaign-ff3ab1e/summary.json`; the production memory lane passed with exact pressure, copy and translation-reuse markers in `build/forty-percent-memory-786c602.log` and the clean full run's `serial-memory` manifest |
| Transactional storage | `build/storage-fault-evidence/1790077462740587000/manifest.json`; 606 production-seam fault images independently decoded, including loss, reordering, both-generation corruption and repeated repair failures. Physical power loss remains outside the model |
| Pure Rust checks | `build/pure-rust-evidence/1790078228643094000/manifest.json`; pinned Miri ran 53 selected production tests and the ownership model explored 3,257,436 sequences / 19,248,492 transitions through depth six. Neither covers the whole kernel |
| Buildable series and compatibility | Four intermediate source commits independently linked validation images in `build/forty-percent-buildable-series/`; [COMPATIBILITY.md](COMPATIBILITY.md) inventories twenty public boundaries and the candidate's rollback/limitations plan |

The complete host workspace run passed **263 Rust tests** (14, 9, 2, 7, 185 and
46 across its test binaries); Python discovery passed **75 tests**. The earlier
100,000 parser-mutation run with seed `20260924`, twelve corpus inputs and 693
prefixes is in `build/forty-percent-parser-stress.log`. The target-appropriate
strict Clippy commands, formatting, documentation links and unsafe inventory
checks pass. The refreshed lexical inventory has **434 sites in 85 source files**;
the latest two-source diff only shifted locations, and an inventory is not an
unsafe-boundary soundness review.

Several failed diagnostic runs remain retained. The full-suite attempt on
`507c4c6` reached network validation but timed out because its required HTTP
proofs never occurred (`build/validation-evidence/1790151186977092000/network/`).
Further targeted runs showed two timing assumptions: the validation shell
rejected a TCP socket that had already reached ESTABLISHED, and the host HTTP
helper read an accepted nonblocking socket as though its immediate `WouldBlock`
were an empty request. `1026634` accepts CONNECTING or ESTABLISHED, explicitly
switches accepted sockets to blocking mode with a bounded read timeout, and adds
a delayed-request host regression. Both network variants then passed in the
full suite. Earlier exact-parser failures and their reruns remain in unique
`build/validation-evidence/` directories; no failing manifest was promoted.

The ordinary persistent volume `build/genos-data.img` was never a fault fixture:
its SHA-256 before and after qualification was
`8175ce14b173824cf267dafa82a9f1284019eb9ce9de9682ac15e047aa27b57a`.
All GenOS test QEMU processes were stopped. The tested image is a normal release
image; `build/image-mode.txt` records its mode and paths.

GenOS remains **Experimental**. Remote required-check publication is still blocked
by the GitHub credential's missing `workflow` scope; there is no PR or independent
reproduction from this campaign. F1.2/F1.4, sharing/DMA/raw-reference lifetime,
F4/F5 shared-state audit, S2 storage growth, general C1–C5 terminal work,
coverage-guided fuzzing, 1000 boots and sustained qualification remain open.
The 40% count does not promote R1, R2 or R3.

## September 23 dependency and format slices

The presentation dependency change is isolated in `7bba26f` and `d396ba9`;
`3e55272` refreshes unsafe source locations without adding/removing a lexical
unsafe construct. The new dependency checker and three host regressions passed,
as did 78 Python checks, 185 kernel library tests, strict reference acceptance
and normal image builds on clean `3e55272`. Its reference log is
`build/fifty-f4-reference-3e55272.log`. The checker rejects direct authority
imports and mutable authority borrows in presentation. This closes the scoped
F4.3 direction rule, not the shared-global or unsafe-boundary audits.

Normal release startup dependency probes on `1be308b` used the exact
`genos-q35-tcg-v3` environment, a snapshot-backed boot image, no persistent
data image and no host test service. Both missing-storage cases and the
NIC-present/no-server case passed with clean source: manifests are
`build/startup-dependency-evidence/1790154609065598000/no-controller/manifest.json`,
`build/startup-dependency-evidence/1790154613151646000/empty-controller/manifest.json`
and `build/startup-dependency-evidence/1790154616967327000/nic-no-server/manifest.json`.
They reached the Ring 3 prompt in about two seconds and completed fresh
`echo`/`net`/`mem` exchanges within four seconds under the declared 60-second
deadline. No host network service or data volume was attached.
The first no-controller exploratory manifest is explicitly `invalid`: the old
harness closed its serial log before its reader thread stopped. The corrected
harness joins the reader before closing logs, records reader failures, and has
two negative/order host tests. No VM was left running.

[ADR-0008](adr/0008-bounded-littlefs-growth.md) now selects pinned littlefs for
the *next* storage provider after comparing the specific console workloads,
space/write amplification, RAM, recovery, reuse/licensing and host tooling.
The [primary-source review](research/2026-09-littlefs-format-decision.md) verified
the tag, static-buffer API, format behavior and important GenOS-side quota and
device-sync obligations. Its status remains Proposed; no C code or new media
format has been integrated. S2.2 and S2.3 remain open.

On clean `1be308b`, 187 kernel library tests, 80 Python regressions, normal
release acceptance and strict debug/release reference acceptance passed after
the serial data guard and prompt state change. The exact strict run is
`build/fifty-reference-1be308b.log`; the unsafe inventory still has 434 lexical
constructs, now across 87 source files after adding a pure prompt-state module.
The output sink accepts printable ASCII data only; terminal controls are sent
only by explicit UI operations. A separate clean `ae143d8` VM fixture changes
only delayed INIT hold output, not the production presenter. It emitted
`BACKGROUND\x1b[2J\rATTACK` while `echo PENDING` was partly typed; the serial
transcript displayed `BACKGROUND?[2J?ATTACK`, restored `genos> echo PENDING`, and
showed one `PENDING` result only after Enter. Its source patch, build/image/
firmware identities, exact QEMU command and serial hash are retained at
`build/terminal-safety-evidence/1790154885972713000/manifest.json`. This closes
the scoped C2 display/encoding item; cursor editing, pasted escape decoding,
shell syntax and general background job ownership remain separate C2/C3 work.
