# GenOS 0.56 verification record

The latest October 10 campaign adds complete native image admission before
allocation, with host/parser and live zero-budget evidence. The dated sections
below retain exact commits, passing gates and the four strict CPU/IRET failures.
Earlier entries retain their original scope and dates.

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
the scoped C2 display/encoding item. Cursor editing and paste decoding are
evaluated separately below; shell syntax and general job ownership remain C2/C3 work.

## September 23 terminal input gate

Source **`264dc0a`** separates bounded serial byte/escape decoding in
`kernel::serial_keys`, fixed-capacity Ring 3 line editing in
`genos_shell::editor`, and plain command classification in
`genos_shell::parser`. The active serial path sends keyboard events only to the
live console process; background waiters cannot steal a typed line. New key
codes are additive under experimental ABI 18. The shell's validation image
remains under its unchanged eight-page RX/one-page data and guarded-stack
layout: the shell package alone uses size optimization, with measured
validation `.text` **28,489 bytes**, 4,279 bytes below the 32 KiB link bound.

`build/terminal-editing-evidence/1790156875159464000/manifest.json` retains
the clean source, reference environment, release image and exact QEMU command,
serial/stderr hashes and ten passing normal-boot exchanges. They cover cursor
insertion/delete, history, Home/End/Right/Backspace, command completion,
bracketed paste without newline submission, invalid escape rejection,
cancellation, an 80-byte full line with rejected suffix, timed-out prefix
cancellation and recovery. Unit tests exercise decoder prefix/paste exhaustion
and editor bounds/history/draft/completion. The complete `cargo xtask test`
suite passed on this exact clean commit in `build/fifty-editor-full-264dc0a.log`,
including storage/network/SDK, all ten construction failure cutoffs, six CPU
protection faults and both normal modes. Strict clean-source reference
acceptance passed in `build/fifty-editor-reference-264dc0a.log`. The refreshed
inventory records **423 lexical sites across 91 files**; moving editing out of
`static mut` access removed eleven lexical unsafe blocks, without making the
remaining sites reviewed or safe.

The terminal still has no quoting, working directory, general named program
launch, file-name completion or job-control signal transfer. Unbracketed raw
paste cannot be distinguished from typed Enter and is documented as such.

## September 23 architecture and test-wiring audit

[ADRs 0009–0012](adr/README.md) record the currently implemented native ABI,
single-BSP scheduling/IRQ division, device/coordinator ownership and GFS2
snapshot contract. Each states the chosen trade-off, denied/failure behavior,
compatibility and rollback path, evidence and limits. They are **Proposed**
records on this local branch, not accepted release contracts. The existing
[compatibility inventory](COMPATIBILITY.md) retains the public-boundary
migration/downgrade plan; [USERSPACE.md](USERSPACE.md) now distinguishes the
actual serial commands from kernel-invoked validation modes and reports ABI 18.

`python3 tools/test_kernel_test_wiring.py` compares every standalone kernel
test file containing `#[test]` with `cargo test -p kernel --lib -- --list`.
All **48** named tests from exception, IPv6, network and storage files were
listed under the production-module paths; a synthetic missing name fails its
negative check. Network and storage use host mock devices only at the driver
boundary while compiling the production protocol/commit modules. This
evidence closes the local F4.5 documentation/wiring item, not an unsafe-site,
shared-global, whole-kernel, remote-CI or independent reviewer audit.

## September 23 construction and teardown audit

Commit `ed8574f` and [CONSTRUCTION_FAILURES.md](CONSTRUCTION_FAILURES.md)
cover the current BSP supervisor split, user-root/page mapping, partial ELF and
stack construction, process reaping, file/process/socket handles and RAM VFS
node construction. A one-page supervisor split now restores its original parent
entries, retires translations, and releases new child tables before returning
an error. Failed process and handle rollback no longer discards teardown
results. Completed supervised children are reclaimed before their slots are
removed. An oversized RAM VFS create/overwrite/append preserves the prior
namespace and bytes; its new host test passed with the 192 kernel library
tests. `SDK.ELF` is loaded directly from the initrd and excluded from the
512-byte RAM VFS seed path.

On this source, `cargo xtask test-memory` passed all ten production process
allocation cutoffs with `leaked_frames=0` in
`build/fifty-construction-memory.log` and `build/serial-memory.log`.
`cargo xtask test-protections` passed six exact-address CPU fault probes on
clean `ed8574f` (`build/fifty-construction-protections-ed8574f.log`).
`cargo xtask test-reference` passed the clean pinned reference check and both
normal debug/release boots
(`build/fifty-construction-reference-ed8574f.log`). The standalone SDK boot
passed (`build/fifty-construction-sdk.log`), and the isolated network lane
passed, including the no-HTTP-server boot at 27,504 ms against its 30,000 ms
budget (`build/fifty-construction-network-retry.log`, retained manifest
`build/validation-evidence/1790159262951016000/network-without-http/manifest.txt`).

The combined `cargo xtask test` command did **not** pass end to end during
this audit. An earlier candidate exposed an SDK boot-seed panic, which was
fixed and verified. The final code's retry timed out in the no-HTTP-server
validation boot at 30,000 ms after
entering its passive listener wait loop. Its failed manifest remains at
`build/validation-evidence/1790159034059994000/network-without-http/manifest.txt`.
The subsequent isolated network rerun passed without changing code or the
budget. This variability remains open under F6/F7 reliability and timing work;
it is not a clean whole-suite claim. F3.4 covers the audited existing
constructor/teardown boundaries, not future C1 mapping/heap growth or DMA/SMP.
The unsafe-inventory diff for this change identified only the two expected
modified `paging.rs` unsafe blocks (root admission and supervisor splitting);
the lexical total remains **423 sites in 91 files**. The reviewed source-context
baseline was refreshed and `check_unsafe.py --check` plus its 11 scanner tests
pass. This is inventory maintenance, not the F4.4 unsafe-boundary review.

## September 23 complete 1000-boot release lane

The clean detached source `7b8caa921d860e7f1904c74577fa13f63550f144`
ran `cargo xtask test-repeat 1000` to exit zero. The last lines are
`NORMAL_BOOT_REPETITION_PROGRESS completed=1000 requested=1000` and
`NORMAL_BOOT_REPETITION_OK completed=1000 requested=1000`. Every fresh QEMU
release boot completed the visible `uname`, launch/status/kill/reap and
persistent-file workflow. No first failure occurred.

The complete log, 1000 distinct manifests and all 2000 serial/QEMU output
files were copied into `build/fifty-repeat-evidence/7b8caa9/`. The log SHA-256
is `200127ce2b0316fb682cacaf90a2eaa54b45f44ca27269ad485833b8e6624772`;
the unchanged release image SHA-256 is
`806f6095d7586020a3455547b4e7c23e90a44eaa89d9210e255a0a66f48c1b59`.
Every manifest reports clean source, `genos-q35-tcg-v3`, the pinned firmware
SHA-256 `33090cc07675baa5190d9f1e84bf5176b33bcbfa9bacac522961150cdb6dbb2a`,
QEMU 11.1.1 and Rust 1.97.0. The test used the disposable
`genos-data-repeat-test.img`, not the user's `genos-data.img`.

`tools/audit_repeat_boots.py` passed on the copied evidence. It requires every
ordinal 1–1000 exactly once in both the ordered log and manifests, a single
final completion marker, distinct run IDs, clean source, one release/profile/
firmware/image identity, and hashes of every retained serial and QEMU log.
Eight host tests reject missing, duplicated, dirty, changed-image, failed and
tampered-log evidence. The runtime source directories (`bootloader`, `kernel`,
`userspace`, `crates`) have no diff from the tested commit on this branch; later
commits in the series update only evidence tooling and documentation.

This closes F6.5's configured fresh-boot lane only. It does not substitute for
F6.6's 10,000 lifecycle cycles and proposed 24-hour single-boot pressure, C5's
72-hour mixed console workload, physical-device qualification or remote CI.

## September 28 bounded CPU context and FP qualification

Source `b9ca7fd` adds private DS/ES/FS/GS selector images, neutral kernel selectors
on entry, initialized/read-back zero FS/GS bases, null LDTR, disabled hardware
debug control, and explicit denial of unowned SYSCALL/SYSENTER entry. The kernel
continues using the CPUID-gated aligned 512-byte FXSAVE64 fallback and soft-float.
The public ABI, IRET frame and storage/ELF formats are unchanged. The register
inventory, ownership, failure and rollback contract is in [CPU_STATE.md](CPU_STATE.md)
and [Proposed ADR-0013](adr/0013-bounded-user-cpu-context.md). Companion `00b7647`
refreshes the lexical source inventory to **436 sites across 92 files**; it does
not establish a whole-kernel unsafe review.

All hardware evidence below identifies clean tested commit
`00b76473bd37381f59dcb2893189ccdb159eb255`, Rust 1.97.0, QEMU 11.1.1,
`genos-q35-tcg-v3` and the pinned firmware digest. Instruction extension cases
name their explicit `max`/Intel-vendor variants. Later documentation commits
do not change the tested runtime source.

### Passing local gates

- Workspace tests passed, including **194 kernel library** and **46 xtask** tests;
  **90 Python** regressions passed. Normal and validation kernel Clippy, the
  remaining workspace/target strict lint commands, Markdown links, test wiring,
  formatting and unsafe-inventory checks passed. The deterministic parser harness
  passed its 12 corpus inputs, 693 truncations and 25,000 mutations. Logs are
  `build/cpu-context-workspace.log`, `build/cpu-context-python-host.log` and
  `build/cpu-context-parser.log`.
- **`cargo xtask test` passed end to end**, including serial persistence/input,
  read-only/recovery/failed storage, network with/without HTTP, standalone SDK,
  all ten process allocation cutoffs, six CPU page-protection faults and normal
  debug/release interaction. Its log is `build/cpu-context-full-qemu.log`, SHA-256
  `4fc1f496ead6bdc86d1dc5d7a85f28c6396b29a09fdcec50e6fd441ca77ba346`.
  The no-HTTP lane completed in **26,154 ms** within its unchanged 30,000 ms
  deadline, manifest `build/validation-evidence/1790598770730387000/network-without-http/manifest.txt`.
- **`cargo xtask test-reference` passed**, with exact environment acceptance and
  both normal images on the clean source. Log `build/cpu-context-reference.log`
  has SHA-256 `39b247c8994d8d8c09ebf667845c13991ff3b7e947b75c0bb113120ee2f070a3`.
  Normal manifests are `build/normal-evidence/1790599041191199000/normal-debug/manifest.txt`
  and `build/normal-evidence/1790599052701768000/release/manifest.txt`.
- The six-process production-loader fixture passed private selectors, FS/GS-relative
  zero-base reads, direct/yield syscalls, **14 actual preemptions**, two pending
  unmasked x87 exceptions across scheduling, exact user #MF, fresh state on reuse
  and frame-baseline reclamation. The retained full serial manifest is
  `build/validation-evidence/1790598627594908000/serial/manifest.txt`, image SHA-256
  `65870f625e071be1cfb4704420b68a8f6c53a86ebd6ea9d82d12df107fa14786`.
  The exact additional marker is
  `USER_CPU_CONTEXT_OK processes=6 selectors=ds,es,fs,gs bases=zero pending_mf=2 fresh=6 reclaimed=true`.
- Separate actual user/kernel **#MF** probes passed their termination/reclamation
  and deliberate-halt contracts. Manifests are
  `build/exception-evidence/user-mf/1790598600000044000/manifest.json` and
  `build/exception-evidence/kernel-mf/1790598809420084000/manifest.json`.
- All **21 admission/ISA/inherited-state cases** have passing isolated evidence
  on this commit. The new cases are
  `build/cpu-feature-evidence/1790598810762730000/isa-syscall/manifest.json`,
  `build/cpu-feature-evidence/1790598827229821000/isa-sysenter/manifest.json` and
  `build/cpu-feature-evidence/1790598843707246000/aux-inherited/manifest.json`.
  The last explicitly poisons/read-backs all three base MSRs and enables a user-code
  breakpoint before invoking the unchanged production reset and reaching the
  normal shell without a fault. The two older absent-FXSR/SSE firmware failures
  remain separate unsupported platform cases; they were not rerun or relabeled.

### Retained failures and qualification gaps

The 18-case existing-CPU campaign returned nonzero on WRPKRU. It delivered the
correct user #UD, but both healthy peers exited before the fault, violating the
unchanged fault-before-peer-exit proof. That failed manifest is retained at
`build/cpu-feature-evidence/1790598838944690000/isa-wrpkru/manifest.json`.
An isolated rerun passed without any code or budget change at
`build/cpu-feature-evidence/1790599079187708000/isa-wrpkru/manifest.json`.
Thus every selected case has a strict passing run, but the initial combined
campaign was not clean; its ordering variability remains F6/F7 work.

Strict user/kernel **#XM qualification failed**. User DIVSS 0/0 returned to exit
254, and the kernel probe printed `KERNEL_EXCEPTION_PROBE_RETURNED`; neither
delivered vector 19. The failed manifests are
`build/exception-evidence/user-xm/1790598615949421000/manifest.json` and
`build/exception-evidence/kernel-xm/1790598827102328000/manifest.json`.
The separate diagnostic replay at
`build/exception-evidence/user-xm/1790598859984439000/manifest.json` also remains
failed, retaining its exact source patch, image identity and `init.elf`. Disassembly
confirms LDMXCSR/STMXCSR/XORPS/DIVSS; exit 253 is produced only when MXCSR is
**`0x1f00` before and `0x1f01` after** the operation. This observes unmasked
invalid status without a delivered trap on the pinned TCG binary. The architecture
contract requires a real #XM; a flag or software-injected vector cannot close it.

Another qualified CPU/emulator/accelerator is needed for SIMD exception delivery
and containment. The strict cases and configured independent jobs remain failing
until that evidence exists. General TLS/debugger interfaces, larger feature/physical
profiles, exhaustive FP/flag interactions and F1.4 nested/return-fault containment
remain open. No R1/R2/R3 or engineering release level advances. The frozen console
checklist remains **50/100**; this closes a bounded F1.2 implementation/evidence
slice, not the entire F1.2 criterion or a new checklist entry.

## September 28 nested entry and return containment

Runtime source `1da63a8aec563ff9874e9b632fd3ec6a04dcffab` adds a single-BSP
assembly entry claim, redirects all five live IST slots while Rust owns its
frame, and halts nested entries and #DF before Rust or CPU-state helpers. Normal
and nonlocal returns restore original tops and clear the claim last. Native
CS/SS/RIP/RSP/RFLAGS validation denies unsupported return state; supported MCE/MCA
CPUs enable/read back CR4.MCE and treat #MC as fatal. The contract and transient
assembly windows are in [FAULT_CONTAINMENT.md](FAULT_CONTAINMENT.md) and
[Proposed ADR-0014](adr/0014-terminal-entry-ownership.md). Seven stacks and fourteen
guards, the public ABI, frame layout and storage formats are unchanged.
The refreshed lexical inventory records **443 sites across 93 files**, without
claiming a whole-kernel unsafe review.

All following VM runs retain clean source identity, Rust 1.97.0, QEMU 11.1.1,
`genos-q35-tcg-v3`, firmware digest, source fixture, image hash and exact command.
Later source commits below change harness contracts, metadata and documentation;
the kernel runtime remains the one introduced in `1da63a8`.

### Passing local gates

- **196 kernel library tests**, **46 xtask tests**, the remaining workspace tests
  and **97 Python regressions** passed. Formatting, strict Clippy for normal and
  validation kernels and every shipped target, documentation links, test wiring
  and unsafe-inventory checks passed. The parser stress harness passed 12 corpus
  inputs, 693 truncations and 25,000 mutations. Logs include
  `build/fault-containment-workspace.log`, `build/fault-containment-xtask-host.log`,
  `build/fault-containment-python-final.log` and `build/fault-containment-parser.log`.
- **`cargo xtask test` passed end to end** on clean
  `03f99aaa3f53915a5b1a7f46a8174b2e3f60913d`: persistence/input, read-only/recovery/
  failed storage, both network lanes, the standalone SDK, all ten process
  allocation cutoffs, six exact-address CPU protection faults and normal
  debug/release interaction. Log `build/fault-containment-full-rerun.log` has
  SHA-256 `cf554c3c735e5cb59d411a90cefb56b334ffc376ee77df9c382edd6b644df682`.
  The no-HTTP lane completed in **23,292 ms** within its unchanged 30,000-ms
  deadline, manifest
  `build/validation-evidence/1790602795718194000/network-without-http/manifest.txt`.
  Its full serial CPU fixture still reports **14 actual preemptions**, private
  selectors, pending #MF, clean reuse and exact frame reclamation at
  `build/validation-evidence/1790602650600806000/serial/manifest.txt`.
- **`cargo xtask test-reference` passed** on clean
  `18c11b64817c90b9ae8ca96e88377110284f6e77`, with exact environment acceptance and
  both normal images. Log `build/fault-containment-reference.log` has SHA-256
  `ee431a63551ae178560e285ab4e6dfba401b637d118211519ce6dab88a018f2f`.
  Manifests are `build/normal-evidence/1790603084328655000/normal-debug/manifest.txt`
  and `build/normal-evidence/1790603095647961000/release/manifest.txt`.
- **All 16 stack cases passed** on clean
  `823a7d4f96863f84089c35067661c8e7d1ad8f9c`: fourteen guard faults, the multipage
  compiler probe and every stack's usage/margin proof. Manifests remain under
  `build/stack-guard-evidence/1790601890773123000/`; the log
  `build/fault-containment-guards.log` has SHA-256
  `848c62b2573cdbb89b1323a474d8ac8c423b98d9b60f2e3e29db818b1244a280`.

### Fault and return matrix

The current nineteen-case harness has **seventeen passing cases** across the
exact clean commits below. Software denials require their own marker and forbid
CPU exception frames; they do not count as hardware #GP/#SS delivery.

| Cases and outcome | Tested source and retained evidence |
| --- | --- |
| IRQ #PF, syscall #GP, same-IST #PF, IRQ NMI/#MC, #MC-handler #GP; all nested terminal paths preserve the active Rust stack | `1da63a8`; `build/fault-containment-evidence/1790601712399422000/{irq-pf,syscall-gp,exception-pf,irq-nmi,irq-mce,mce-gp}/manifest.json` |
| Initial bad CS at IRET, real #DF, first NMI/#MC; deliberate terminal/kernel halt with guest still running | `1da63a8`; same campaign's `{initial-cs,double-fault,nmi-idle,mce-idle}/manifest.json` |
| Invalid initial flags halt before user entry; user NT is rejected, terminated and reclaimed while healthy peers finish | `1da63a8`; same campaign's `{initial-flags,user-nt}/manifest.json` |
| Syscall bad CS faults at exact IRET with kernel SS null | `333fad9a858caed6b92d3c561a40814cd31c42d3`; `build/fault-containment-evidence/1790602406603034000/syscall-cs/manifest.json` |
| IRQ bad CS faults at exact IRET after entry release | `333fad9`; `build/fault-containment-evidence/1790602444242311000/irq-cs/manifest.json` |
| Successful IRET inside the active NMI handler unblocks a second external NMI; child uses the terminal stack and never enters Rust | `333fad9`; `build/fault-containment-evidence/1790602480168845000/nmi-reentry/manifest.json` |
| Invalid initial RSP and SS halt before user entry, with no fabricated CPU exception | `18c11b6`; `build/fault-containment-evidence/1790603054161364000/initial-rsp-reject/manifest.json` and `build/fault-containment-evidence/1790603069309838000/initial-ss-reject/manifest.json` |

External events retain the complete QMP transcript: `inject-nmi` delivers vector
2, and the accepted monitor `mce 0 0 0xb200000000000000 0x5 0x0 0x0` delivers vector
18 after production CR4.MCE admission/readback. These are external emulator CPU
events, not software INT substitutes. Terminal evidence checks exact vector,
parent/error/CR2, stack address and active-window order. Return faults require
saved RIP to equal the linked IRET site. Every fatal proof also requires QEMU to
remain running after the explicit halt, rejecting reset, exit or a paused guest.

### Retained failures and corrected assumptions

The first seventeen-case campaign on `1da63a8` returned nonzero: twelve cases
passed and five failed. All failed manifests and serial/QMP logs remain under
`build/fault-containment-evidence/1790601712399422000/`.

Two failures remain strict qualification gaps:

- **`nmi-iret`:** the first real NMI reaches an invalid-CS IRET and a nested kernel
  #GP (`error=0x38`, parent 2) halts on the terminal stack. QMP accepts the second
  NMI, but no second vector 2 arrives before the unchanged 60-second deadline.
  QEMU 11.1.1's [IRET helper source](https://gitlab.com/qemu-project/qemu/-/raw/v11.1.1/target/i386/tcg/seg_helper.c)
  clears NMI blocking only after the return helper succeeds. This accounts for
  the observed difference from [Intel's instruction contract](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2a-manual.pdf),
  which unblocks NMI even when IRET faults. The separate successful-IRET control
  passes second-NMI containment; it does not close this failed-return requirement.
- **`initial-rip`:** corrupting the post-validation return RIP to
  `0x0000800000000000` delivers #GP after the target context is loaded: CPL 3,
  CS `0x33`, saved RIP equal to the bad address. It terminates all three fixture
  processes and fails isolation; the required kernel-origin #GP at IRET is absent.
  The same QEMU source commits target RIP/CS without the required canonical-RIP
  check. The strict saved-IRET-site requirement remains; no later fault is relabeled
  as a correct return-instruction fault.

The other three initial failures were fixture/parser mistakes:

- **`syscall-cs`:** the actual #GP and IRET address were correct, but the parser
  incorrectly required interrupted kernel SS `0x10`. Long-mode interrupt entry
  sets it to null. The corrected exact-null requirement has its own negative
  host test and the clean passing rerun listed above. The original failure remains.
- **`initial-rsp` / `irq-rsp`:** the fixtures incorrectly assumed loading a bad
  target RSP must produce #SS at IRET. The Intel operation loads that value;
  #SS's stack-address checks concern memory accesses. The observed later user
  #PF and kernel #GP are retained, without claiming these runs prove an emulator
  defect or the intended return-instruction requirement. These cases were retired
  from the active matrix and replaced with production software rejection of
  invalid initial RSP/SS; they were not retroactively marked passed.

The first full suite on `823a7d4` also failed its 120,000-ms network deadline:
DNS/HTTP/API/timeout/diagnostic proof records were absent even though passive TCP
and the console finished. Manifest
`build/validation-evidence/1790602053543341000/network/manifest.txt` and log
`build/fault-containment-full.log` are retained. The network lanes passed unchanged
in isolation at `build/validation-evidence/1790602259627794000/network/manifest.txt`
and `build/validation-evidence/1790602285933206000/network-without-http/manifest.txt`;
the later complete suite passed too. Concurrent VM load is a possible contributor,
not an established root cause. No timeout, marker, ordering or network budget was
relaxed. An additional test attempt started with uncommitted evidence-parser
changes was stopped and retained in `build/fault-containment-uncommitted-attempt.log`;
it is excluded from qualification. The successful retry began on clean `03f99aa`.

Another qualified emulator/CPU is needed for the two remaining IRET boundaries,
as well as the earlier #XM gap. The nineteen independent workflow jobs retain
failures, but workflow publication and remote execution remain unestablished.
No R1/R2/R3 or engineering release level advances. **F1.4 remains unchecked** and
the frozen checklist remains **50/100**. Physical corruption, terminal-stack/UART
failure, unrecoverable CPU shutdown, second #MC with MCIP active, arbitrary storms,
SMP and general NMI/machine-check recovery remain excluded.

## October 10 CPU/scheduler extraction and published verification

Clean source `d603c555b08490d204a04bf5af554c0bc89080bd` extracts the native
twenty-word CPU frame/initialization/return admission into `cpu_context`, and the
bounded process/socket round-robin policy into `scheduler`. Process-owned bounds,
active-root checks, dispatch and cleanup stay with the coordinator. Five context
tests cover exact assembly offsets, fresh registers, all 65,536 selector values,
each flag bit and executable/stack boundaries. Five scheduling tests cover every
ready subset, bounded equal service, idle/invalid selections, wakeup batches and
empty/singleton/overflow boundaries. No ABI or frame layout changed.

Local results on Rust 1.97.0/macOS arm64:

- **288 workspace tests passed**, including 206 kernel tests. The standalone
  parser CLI tests passed; seed zero completed 100,000 mutations plus the twelve
  retained seeds and 693 truncations. These remain mutation stress, not
  coverage-guided fuzzing.
- Strict lint passed for host tooling/ABI, bootloader, kernel normal and validation
  features, and userspace normal/validation targets. Normal release and validation
  images linked. Formatting, module dependencies, documentation links and the
  unchanged 443-site unsafe inventory passed.
- `cargo xtask test-release` and the **complete `cargo xtask test` suite passed**:
  interactive normal debug/release boots; file/serial and storage recovery/failure
  paths; network and no-HTTP policy; external SDK execution; allocation rollback;
  and all six CPU page-protection probes. Per-run manifests remain under
  `build/normal-evidence/`, `build/validation-evidence/` and
  `build/exception-evidence/`; combined logs are retained in
  `build/october10-foundation/`.
- The complete nineteen-case containment matrix had **seventeen passes and two
  failures**, `nmi-iret` and `initial-rip`, with all per-case manifests/logs/patches
  under `build/fault-containment-evidence/1791664971063150000/`. The separate user
  #XM run timed out after its probe returned and the isolation check failed;
  its evidence remains under
  `build/exception-evidence/user-xm/1791664863842719000/`. These are retained
  qualification failures, not green results.

These VM runs used **QEMU 11.1.2**, with the pinned machine/CPU arguments and
matching firmware digest. Every manifest records `reference_environment_match=false`
and `reference_mismatches=qemu_version`. The actual reference preflight rejected
this version. These are portability results; the `genos-q35-tcg-v3` profile was not
changed and no new exact-reference qualification is claimed.

The OAuth credential now has `workflow` scope. The branch was published as
[draft PR #9](https://github.com/ErzenXz/GenOS/pull/9). All **sixty GitHub Actions
checks completed on exact head `d603c55`: 56 passed and four failed**:

- [CI](https://github.com/ErzenXz/GenOS/actions/runs/38084730259) passed all sixteen
  jobs, including the complete build/boot lane and all six independent boot-map
  cases.
- [Exception entry](https://github.com/ErzenXz/GenOS/actions/runs/38084730268)
  passed its source/evidence jobs and ten exception cases; user/kernel #XM both
  failed their exact exception-delivery deadlines.
- [Foundation containment](https://github.com/ErzenXz/GenOS/actions/runs/38084730265)
  passed 28 cases; `initial-rip` reported failed containment and `nmi-iret` timed
  out without its required second-NMI result. All matrix jobs ran independently.

Check-run metadata binds these results to PR head `d603c55`; the archived source
manifests correctly identify GitHub's synthetic merge commit
`0f11f21f709d432d2e1a99718ba545ad55dfa428`. Its parents are the unchanged `main`
`a4432b8` and `d603c55`. Both the synthetic merge and reviewed head have tree
`fdf1299b4bdce3a35d33c89d85a14de1f4a6d61b`; a fetched `refs/pull/9/merge` and
`git diff --exit-code d603c55 FETCH_HEAD` independently confirmed identical source
content. This is exact reviewed-tree evidence with two explicit commit identities.
The follow-up workflow change checks out the PR head explicitly, records actual
HEAD/tree separately from the event commit, and retains the full per-run QEMU
manifests. Its next-head remote results must be tracked separately.

The failed remote logs and their downloadable fixture/serial/manifest artifacts
remain retained. Linux jobs are independent clean portability builds, not another
reviewer's reproduction of the pinned macOS reference environment.

`main` now requires all sixty context names bound to GitHub Actions App ID 15368,
strict base freshness and administrator enforcement. Force pushes/deletion are
disabled; no approval-count rule was added. The API mutation was read back and
compared field-for-field. The [versioned policy and read-only audit](CI_POLICY.md)
preserve this contract, exact-head execution and failure outcomes; negative tests
reject weakening, missing/duplicate/stale/wrong-app/skipped results and overwrites.
The execution audit at `build/ci-policy-evidence/d603c55-complete-audit/` reports
`required=60 executed=60 failed=4`; `--require-success` fails on those four lanes.
This closes **F0.3 only**, moving the original checklist to **51/100**.

F0.1 remains open: no merge, green candidate, independent reference review, R1/R2/R3
designation or engineering release promotion occurred. The remaining F1 CPU
qualification failures, F3/F4/F5 ownership work, coverage/soak runs, storage and
application/console roadmap requirements still apply. Historical missing-workflow
statements above describe earlier attempts and are superseded by this dated result.

## October 10 native image admission before allocation

Kernel change `3648625722ed0a3067a85eb8cafb88d2f0972942` moves whole-image native
ELF admission into the allocation-free `kernel::native_image::Plan`. Its private
ten-page plan borrows exact file slices and validates every segment before
`build_process` creates an owner or root. The paging adapter retains partial-root
rollback for later allocation/mapping errors. Fixed addresses, the eight-page
executable limit, sixteen initial ABI data bytes and executable-memory entry
policy remain unchanged. This advances F4.1; process state, mapping/stack
construction, lifecycle, syscalls and remaining handle decomposition stay open.

Host checks cover the loader source and corrected harness. The complete VM
suite used clean source `08552989539962a4bc7f8b6e45d57e6db7fa4fdf` (the same
kernel plus a corrected evidence ordering rule):

- **296 workspace tests passed**, including 214 kernel tests and eight new native
  admission tests. All 107 Python tests and the two parser CLI tests passed.
  Strict lint passed for the changed kernel host/bare-metal normal/validation,
  xtask and parser targets. Formatting, documentation links, module direction
  and the reviewed 443-site/96-source lexical inventory passed.
- Seed zero passed **100,000 mutations**, fourteen retained corpus inputs and
  **17,109 proper-prefix truncations**. The new `native-elf` target calls the
  production admission API and checks borrowed slices, unique bounded pages,
  W^X, executable/data limits and entry mapping. Valid and late-W+X seeds are
  retained; both freshly linked INIT and shell images were accepted in replay.
  This remains deterministic mutation stress; F6.2 coverage-guided work is open.
- The **complete `cargo xtask test` suite passed**: storage create/restore and
  read-only/recovery/failure phases, real serial input, network and no-HTTP
  policy, external SDK execution, memory rollback, all six exact-address CPU
  protection probes and interactive normal debug/release boots. The latter
  exercise process launch/status/kill/reap and persistent file I/O. All eighteen
  per-run manifests passed; their paths/hashes are indexed in
  `build/native-image-evidence-index.json`. Combined output remains in
  `build/native-image-system-tests-corrected.log`.
- The memory manifest at
  `build/validation-evidence/1791666492084345000/serial-memory/manifest.txt`
  requires exact `NATIVE_IMAGE_PREFLIGHT_OK` results: late W+X rejection with
  allocator budget zero, valid-image OOM with that same budget, and an unchanged
  live-frame baseline. All ten later process-construction allocation cutoffs
  still restore the baseline. Normal images exclude the new test marker.

The first local run on `3648625` failed because the new harness ordering rule
incorrectly placed preflight before memory hygiene. The guest emitted the genuine
preflight result after hygiene/IRQ restoration, and the harness rejected that
order. The failed manifest/log remain at
`build/validation-evidence/1791666303709328000/serial-memory/`. The follow-up
`0855298` fixes the order and tests omissions, duplicates, swaps, embedded proofs
and forged fields; the complete rerun above passed. This was a harness defect,
not a passed run or a relaxed guest budget.

These local VM runs use QEMU 11.1.2 and matching firmware. Their manifests retain
`reference_environment_match=false` for the QEMU version difference; the frozen
11.1.1 profile remains unchanged. No new exact-reference qualification is claimed.

Published head `9efd3faf4621e98da87d8567d0e76cc828db47e8` completed all sixty required
checks: **56 passed, four failed**. Its failed fixture artifacts now identify that
exact PR head directly; they are retained under `build/remote-evidence/9efd3fa/`.
The read-only audit at `build/ci-policy-evidence/9efd3fa-complete-audit/` reports
`required=60 executed=60 failed=4`; the retained required-success replay fails.

For native-loader head `0855298`, [exception entry](https://github.com/ErzenXz/GenOS/actions/runs/38086301918)
and [foundation containment](https://github.com/ErzenXz/GenOS/actions/runs/38086301911)
again retain user/kernel #XM, `initial-rip` and `nmi-iret` failures. All four
artifacts explicitly name `0855298` and remain under
`build/remote-evidence/0855298/`. The main
[CI workflow](https://github.com/ErzenXz/GenOS/actions/runs/38086301909) passed all
sixteen jobs, including the complete remote QEMU suite. The final read-only audit
at `build/ci-policy-evidence/0855298-final-audit/` reports all sixty required
checks executed: **56 passed and four failed**. Its required-success replay fails;
the earlier pending audit is retained separately. Passing remote QEMU artifacts
remain under `build/remote-evidence/0855298/passing-qemu/`. These Linux runs are
independent portability execution, not a pinned-reference reviewer sign-off.
Draft PR #9 remains unmerged and the roadmap stays **51/100**, Experimental.
No R1/R2/R3 or whole-roadmap completion is claimed.

## October 10 scoped current-process access and storage quarantine

Committed head `5ec29419f07e94018a6cae07a9c557328c9dffd6` replaces the raw global
mutable-process reference with a scoped `ContextSlot` publication and borrow
boundary. The admitted BSP suspends its owner reference through the user slice;
assembly field pointers preserve that publication pointer's provenance. Rust
entry callbacks require IF clear and reject reentrant access before dereference.
Syscall action handling is separate from the entry adapter. This advances part
of F4.1/F4.2/F5.3; it does not audit every mutable global or enable SMP.

- All **303 workspace unit tests**, plus a compile-fail borrow-escape doctest,
  **108 Python tests** and strict lint for the affected host/target builds passed.
  The reviewed lexical inventory records **454 sites across 97 sources**.
- The complete pinned Miri campaign passed **60 selected tests**, including seven
  scoped-slot cases with strict provenance and assembly-field pointer sequencing.
  Native allocator stress also passed. The clean-source manifest is
  `build/pure-rust-evidence/1791667448914843000/manifest.json`; hardware/assembly
  and the wider kernel are outside its scope. The ownership model was unchanged
  and was not rerun in this Miri-only campaign.
- Real Ring 3 syscall reentry reached exactly one production
  `CPU_LOCAL_ACCESS_REJECTED policy=halt`, without executing the nested callback,
  resuming boot, faulting or stopping the VM. The passing manifest is
  `build/fault-containment-evidence/1791667450075041000/syscall-borrow/manifest.json`.
  This is software ownership denial. Its first manifest used an overly broad
  generic CPU-fault scope label; the harness metadata has since been corrected.
- The complete `cargo xtask test` suite passed all eighteen per-run manifests on
  clean `5ec2941`. The index and hashes are in `build/5ec2941-system-evidence-index.json`;
  combined output is `build/context-slot-system-tests.log`. Storage/serial,
  network/no-HTTP, SDK, memory preflight/ten rollback cutoffs, six CPU protections,
  and normal debug/release terminal/process/file workflows all executed.

Actual `main` protection was strengthened from sixty to **sixty-one** required,
App-bound checks without changing any other protection. The readback is retained
at `build/ci-policy-evidence/context-slot-policy-1791667447650837000/`.
[Remote CI](https://github.com/ErzenXz/GenOS/actions/runs/38087489395),
[exception entry](https://github.com/ErzenXz/GenOS/actions/runs/38087489386) and
[foundation containment](https://github.com/ErzenXz/GenOS/actions/runs/38087489400)
completed all sixty-one required lanes on `5ec2941`: **56 passed, five failed**.
The new borrow lane passed; existing user/kernel #XM, `initial-rip` and `nmi-iret`
qualification still failed. Selected success/failure artifacts remain under
`build/remote-evidence/5ec2941/`; the retained audit at
`build/ci-policy-evidence/5ec2941-first-audit/` verifies enforcement/execution,
not a green candidate.

The fifth failure, build/boot, exposed a distinct recovery-consumer bug. On a
fresh volume, a real `ATA_COMMAND_TIMEOUT phase=complete` caused production
`PERSISTENT_STORAGE_QUARANTINED outcome=not-published`. The kernel correctly
published a 75-byte read-only report; the validation shell accepted only short
legacy records and exited 243. Exact failed serial/image/source evidence remains
at `build/remote-evidence/5ec2941/first-ci-qemu/validation-evidence/1791667524460902066/serial-persistence-create/`.
The later shell fix recognizes both complete quarantine outcomes and checks
persistent write/manage/namespace denial plus RAM-file readability, without
requiring seed files that fresh-volume creation never acknowledged. Three host
parser tests reject incomplete, unknown, duplicate and inconsistent records.
Kernel quarantine and ATA polling budgets are unchanged; the timeout's underlying
cause is not fixed by this consumer correction.

The new independent storage-create-failure fixture injects an error only at
`AtaIo::write`. Actual branch protection was strengthened to **sixty-two** required
checks; full before/after readback, request and response hashes remain at
`build/ci-policy-evidence/storage-startup-policy-1791668406611523000/`.
Its first local attempt on `cb380ab` failed before boot because the plain archive
had no Git metadata required by the serial runner. That failed manifest remains
at `build/storage-startup-evidence/1791668393276058000/manifest.json`; it is not
runtime failure or passing qualification. The disposable checkout correction
retains the archived commit identity and tests that uncommitted/untracked caller
content cannot enter the fixture.

Local QEMU remains 11.1.2 with matching firmware and explicit reference mismatch;
these are portability runs, not pinned-reference or independent-review sign-off.
The roadmap remains **51/100**, Experimental, with draft PR #9 unmerged.

On clean shell-fix head `cb380ab976dd99c459d7413502d927e1bfa8dee3`, all
**306 workspace unit tests** plus the borrow-escape doctest passed; the complete
system suite again passed all eighteen per-run manifests. The latter are indexed
with hashes in `build/cb380ab-system-evidence-index.json`, with combined output
in `build/storage-startup-system-tests.log`. Strict host, normal shell and
validation shell/kernel lint passed. The reviewed inventory now contains
**454 sites across 98 sources**, with no additional unsafe construct in this fix.

The disposable-checkout correction on `18fc1d5` booted successfully and its inner
serial runner passed, but the outer wrapper incorrectly rejected the validation
boot's three deliberate earlier CPU faults. Its failed manifest remains at
`build/storage-startup-evidence/1791668666156778000/manifest.json`. The corrected
wrapper validates exactly the expected user #PF and two pending-x87 #MF frames,
matching termination identities and order, all before the storage injection;
any extra, malformed, reordered or later fault fails. The original failures
are retained rather than relabeled as passes. **114 Python tests** now pass,
including immutable checkout identity and these deliberate-fault boundaries.

A fresh run passed on clean
`f00c130f022b7ac2e123da7d43b6a67c8675ffa6`. Its manifest is
`build/storage-startup-evidence/1791668762445626000/manifest.json` and retains the
single-method fixture diff, archive hash, environment, command, per-run image
hashes, serial log and fresh runner nonce. The guest proves one failed fresh-volume
write, not-published quarantine, persistent mutation denial, readable session RAM,
completed headless validation and a responsive Ring 3 terminal. No healthy durable
creation/restore or later fault is accepted. A separate live scoped-borrow rerun
passed on `18fc1d5` with the corrected software-denial scope label at
`build/fault-containment-evidence/1791668667329612000/syscall-borrow/manifest.json`.
These are bounded regressions, not physical-device, complete CPU or release
qualification.

## October 10 immutable boot-image publication

`b7ef0562f2815220a78f1f9a8e32c4e1f509aabd` replaces INIT/shell global integer
address/length pairs with `boot_image::ImageSlot`. A single atomic claimant
publishes the original immutable boot-lifetime slice; readers acquire a complete
pointer/length pair without waiting. Replacement is rejected, and both image
registrations validate complete native admission before publication. Shell boot
now rejects invalid registration before any user launch. Each actual launch
still validates its plan and retains existing failure cleanup.

Five host tests cover absence, original pointer/length identity, replacement
refusal, empty publication and competing publishers with different lengths.
A compile-fail doctest rejects temporary input. The only shared-slice
reconstruction preserves pointer provenance and is documented at its boundary;
eight former adapter unsafe blocks are replaced by one, with no new unsafe
Send/Sync. After source review the lexical inventory records **447 sites across
99 source files**. This is a scoped F4.1/F4.2 improvement, not completion of the
mutable-global audit or kernel SMP support.

- **311 workspace unit tests**, **two compile-fail doctests**, **114 Python tests**
  and strict kernel host/normal/validation target lint passed. Logs remain in
  `build/boot-image-host-tests.log`, `build/boot-image-python-tests.log` and
  `build/boot-image-*-clippy.log`.
- The immutable pinned Miri campaign passed **65 selected tests**, including all
  five image-slot cases and all seven scoped-process cases, with strict provenance
  and seed one. Native allocator stress also passed. Its exact source/archive,
  toolchains, commands, per-module counts and hashes are retained at
  `build/pure-rust-evidence/1791669043221279000/manifest.json`.
  The concurrent case covers its recorded schedule, not all interleavings.
  The bounded ownership-model lane was unchanged and not rerun.
- The production fresh-volume failure regression passed with the typed images;
  its manifest is `build/storage-startup-evidence/1791669043221279000/manifest.json`.
  It retains one deliberately failed device write, sticky not-published quarantine,
  denied persistent mutations, readable RAM session data and responsive shell
  under the unchanged runner deadline and nonce contract.

- The complete `cargo xtask test` system suite passed all **eighteen** per-run
  manifests on clean `b7ef056`. Hashes/paths are indexed in
  `build/b7ef056-system-evidence-index.json`, with combined output in
  `build/boot-image-system-tests.log`. Both image registrations execute through
  storage create/restore/read-only/recovery/failure, serial, network/no-HTTP,
  external SDK, ten construction allocation cutoffs, all six protection probes
  and normal debug/release process/file workflows. No source changed during
  the full campaign.

The earlier storage-fix head `cb380ab` completed all sixty-two required checks:
**57 passed and five failed**. Its new fixture failed before boot on missing Git
metadata; the main remote build/boot suite passed. The retained final audit is
`build/ci-policy-evidence/cb380ab-final-audit/`, with passing main QEMU artifacts
under `build/remote-evidence/cb380ab/passing-qemu/` and the failed storage fixture
under `build/remote-evidence/cb380ab/storage-startup/`.

Published head `91695700a3bc0e99d39b21e5147d799710f90e24` completed all sixty-two
required checks: **58 passed and four failed**. Its
[main CI](https://github.com/ErzenXz/GenOS/actions/runs/38088962062) passed every
job, including the full remote build/boot suite. The corrected storage-create
failure and scoped-borrow lanes both passed in
[foundation containment](https://github.com/ErzenXz/GenOS/actions/runs/38088962097).
Strict `initial-rip`/`nmi-iret` and
[user/kernel #XM](https://github.com/ErzenXz/GenOS/actions/runs/38088962076)
qualification still failed. Selected success/failure artifacts and passing main
QEMU artifacts remain under `build/remote-evidence/9169570/`. The read-only audit
at `build/ci-policy-evidence/9169570-final-audit/` verifies enforcement and complete
exact-head execution, reports all four failures and does not certify green
verification. Its required-success replay is retained separately and fails.

The newer typed-image change has the local qualification above; remote execution
of its published head must be evaluated separately. Local QEMU 11.1.2/firmware
identity and mismatched reference status remain explicit. No pinned-reference,
independent-review, general SMP, complete F4/F5 or release qualification is
claimed. Roadmap remains **51/100**, Experimental, and PR #9 stays draft/unmerged.

## October 11 CPU-local paging publication and refusal

`d451c63efaaa1e8e6dd2d66a979197f220a0db06` replaces three raw paging globals
with `cpu_paging::CpuPaging`. Initialization publishes a complete permanent/active
root pair only after checking the actual CR3. Bootstrap and sealed phases are
irreversible; bootstrap mapping/guard edits close when sealing succeeds, and
private root admission requires sealing. Every root activation retains the live
grant and interrupt exclusion checks, validates admission before MOV CR3, and
checks the resulting CR3 before publishing the active root. A refused observation
halts with IF clear before dispatch, reclamation or interrupt restoration.

The host module contains no unsafe code and grants no page-table authority. Its
seven tests cover initialization, competing publishers, permanent-root identity,
phase boundaries, invalid roots, mismatched observations and repeated switching.
The competing-publisher case covers its recorded schedule, not every interleaving.
The adapter still requires one owning CPU and excludes NMI/emergency mutation;
this does not establish SMP support. Eight former unsafe blocks were removed.
The reviewed lexical inventory now records **439 sites across 100 source files**.

- **318 workspace unit tests**, **two compile-fail doctests**, strict kernel
  host/normal/validation target lint and the complete system suite passed on clean
  `d451c63`. All **eighteen** per-run system manifests are indexed with hashes in
  `build/d451c63-system-evidence-index.json`; combined output is retained in
  `build/cpu-paging-system-tests.log`. The campaign includes storage creation,
  restore, read-only/recovery/failure, serial, network/no-HTTP, external SDK,
  construction preflight and ten allocation cutoffs, all six protection probes,
  and normal debug/release process/file workflows. Source remained unchanged
  during the campaign.
- Pinned strict-provenance Miri passed **72 selected tests**, including all seven
  paging-state tests, on clean `d451c63`. Native allocator stress also passed.
  Exact source/archive, toolchain, commands, per-module counts and hashes remain
  at `build/pure-rust-evidence/1791670144159484000/manifest.json`. The separate
  ownership-model lane was unchanged and was not rerun.
- **120 Python tests** pass after the refusal-oracle refinement in
  `28da1b84235c166105686a0287c8473a74631b92` (119 before that refinement).
  Logs are `build/cpu-paging-host-tests.log`,
  `build/cpu-paging-python-tests-final.log` and
  `build/cpu-paging-*-clippy.log`. No Rust source changed after `d451c63`.

The new disposable active-root fixture passed first on clean `d451c63`, with
manifest `build/cpu-paging-evidence/1791670144159483000/manifest.json`, and again
with the stricter oracle on clean `28da1b8`, with manifest
`build/cpu-paging-evidence/1791670507348493000/manifest.json`. It changes only the
observation supplied to the production publisher after a real private CR3
activation. Actual CR3 must equal the target; the injected observation differs
by exactly one page, and the previous root must differ from the target. The
only accepted serial tail after arming is the exact hardware-mismatch halt
record. Partial preemption, syscall, exit, image launch or other continuation
fails. QMP subscribes before starting the paused VM and retains raw commands and
messages; exactly one resume and a final running status are required, and reset,
shutdown, stop, watchdog and guest-panic events fail. Buffered messages and guest
exit are checked again after the final status. This demonstrates software refusal
of a falsified observation after real activation, not a hardware CR3 failure or
CPU exception qualification.

The production fresh-volume write-failure regression also passed on clean
`28da1b8`: `build/storage-startup-evidence/1791670507345838000/manifest.json`.
It retains sticky not-published quarantine, denied persistent mutations,
readable session RAM and responsive Ring 3 shell under the existing nonce and
deadline contract. The underlying remote ATA timeout cause remains unresolved.

The independent `Foundation active-root-observation` lane raises the versioned
required-check contract to **63**. Main branch protection was updated and read
back with every other policy field preserved; before/request/response/after
records and hashes remain in
`build/ci-policy-evidence/cpu-paging-policy-1791670145255164000/`.

The previous published head `1e07788701bca9175d505939ccbfa827a1250f0f` completed
all **62 then-required checks: 58 passed and four failed**. Its
[main CI](https://github.com/ErzenXz/GenOS/actions/runs/38089505340) passed every
job, including the complete remote build/boot suite. Scoped-borrow and
fresh-volume failure passed in
[foundation containment](https://github.com/ErzenXz/GenOS/actions/runs/38089505312),
while strict initial-RIP/NMI-IRET and
[user/kernel #XM](https://github.com/ErzenXz/GenOS/actions/runs/38089505320)
qualification failed. Artifacts remain under `build/remote-evidence/1e07788/`.
The retained read-only audit at `build/ci-policy-evidence/1e07788-final-audit/`
uses that head's historical 62-check contract and verifies policy/execution,
not green verification. It predates the 63-check policy update; the new head's
remote execution must be evaluated separately.

Local QEMU 11.1.2 has the retained firmware identity but differs from the frozen
11.1.1 reference profile. These manifests explicitly record that portability
mismatch. No reference profile, strict CPU requirement or release level was
relaxed. Complete F4/F5, independent review, CPU qualification and later roadmap
work remain open: **51/100**, Experimental, with PR #9 draft/unmerged.

## October 11 IRQ-shared input ownership

`3ed61890818a2c2ea489bee67a6ffcc97ab5e90f` groups the legacy PS/2 queue, keyboard
decoder, partial mouse packet, pointer and bounds behind one private scoped
adapter. IRQ and coordinator operations, including all observations, mask IRQs
before publishing the permanent original pointer through `ContextSlot`.
Higher-ranked access cannot return a state borrow; both guards end before prior
IF restores. Reentry refuses before dereference rather than waiting. IRQ entry
and optional boot-only controller configuration unconditionally require IF
clear. IRQ work remains at most one ready byte, polling at most eight; the
boot-only mouse configuration retains bounded waits and its existing missing
error-propagation limitation. NMI/emergency input access and other CPUs remain
excluded. This is not an SMP lock or completion of F4/F5.

The pure `input_state` module has six tests covering absent-byte status,
interleaved keyboard/mouse state, overflow ordering with pointer progress,
motion/button coordinates, degenerate dimensions and integer-boundary motion.
Pointer initialization now clamps degenerate dimensions and movement saturates
before clamping. The reviewed inventory records **433 sites across 101 sources**:
five unsafe blocks and one unsafe helper removed, with no new unsafe Send/Sync.

- **324 workspace unit tests**, **two compile-fail doctests**, **125 Python tests**
  and strict kernel host/normal/validation target lint passed. Logs are
  `build/input-ownership-host-tests.log`, `build/input-ownership-python-tests.log`
  and `build/input-ownership-*-clippy.log`. An initial harness self-test found
  duplicate fixture installation was accepted; the fixture now refuses a second
  installation before mutation. That failed self-test remains separately in
  `build/input-ownership-harness-tests.log`; the final full Python suite passes.
- Pinned strict-provenance Miri passed **78 selected tests**, including all six
  input-state cases, on clean `3ed6189`. Native allocator stress also passed.
  Source/archive, toolchain, exact commands and per-module log hashes remain at
  `build/pure-rust-evidence/1791671056479360000/manifest.json`. The separate bounded
  ownership-model lane was unchanged and was not rerun.
- The complete system suite passed all **eighteen** manifests on clean `3ed6189`:
  storage creation/restore/read-only/recovery/failure, serial, network/no-HTTP,
  external SDK, memory preflight/ten allocation cutoffs, all six actual protection
  probes and normal debug/release process/file workflows. Source did not change
  during the campaign. Paths/hashes are in `build/3ed6189-system-evidence-index.json`,
  with combined output in `build/input-ownership-system-tests.log`.

The real delayed-input fixture passed on clean `3ed6189`, with manifest
`build/input-irq-evidence/1791671056479347000/manifest.json`. QMP subscribed before
guest startup and acknowledged one keyboard `a` down event during a deliberately
extended production input borrow. A serial handshake then released the window.
While IF was clear, the guest observed a ready keyboard byte, unchanged IRQ count
and empty queue. After the adapter retired its borrow/publication and restored
IF, the actual production IRQ delivered exactly one `a`. Peek/pop/depth proved
queue retirement; nested coordinator observations/polling preserved an already
clear IF and the outer enabled state. The fixture then halted, with a final
running QMP status and no reset, stopped VM, fault or continued boot. Its retained
diff inserts only the window and call, leaving controller reads, decoder, queue,
adapter, PIC acknowledgement and entry assembly intact. This covers one delayed
keyboard schedule, not mouse hardware, exhaustive nesting, complete workload
latency or allocator/device-completion schedules. See
[input ownership](INPUT_OWNERSHIP.md) for caller obligations and scope.

`Foundation input-irq-deferred` is the new independent required lane, raising
the contract to **64**. The initial policy update succeeded on the server, but
its wrapper rejected GitHub's retained check-array order; that failed wrapper
manifest remains at
`build/ci-policy-evidence/input-irq-policy-1791671341105558000/manifest.json`.
A separate fresh readback verifies all 64 app-bound checks and every policy
field, comparing only the two order-insensitive check arrays canonically. Its
before/request/response/after evidence and hashes remain at
`build/ci-policy-evidence/input-irq-policy-readback-1791671378516534000/`.
No other enforcement field changed; strict freshness and administrator
enforcement remain required.

The previous published head `656fe0e4926963e380060803e91e07d6866ebf05` completed
all **63 then-required checks: 59 passed and four failed**. Its
[main CI](https://github.com/ErzenXz/GenOS/actions/runs/38090919613) passed every
job, including the complete remote system suite. The new root-observation,
scoped-borrow and fresh-volume failure lanes passed in
[foundation containment](https://github.com/ErzenXz/GenOS/actions/runs/38090919577).
Strict initial-RIP/NMI-IRET and
[user/kernel #XM](https://github.com/ErzenXz/GenOS/actions/runs/38090919673)
still failed. Artifacts remain under `build/remote-evidence/656fe0e/`; the
read-only exact-head audit at `build/ci-policy-evidence/656fe0e-final-audit/`
verifies policy and execution using that historical 63-check contract, not green
verification. Its required-success replay fails and remains at
`build/ci-policy-evidence/656fe0e-required-success/`. Both predate the 64-check
policy update. The input change's remote execution must be evaluated separately.

Local QEMU 11.1.2 remains portability evidence with the retained firmware hash;
the frozen 11.1.1 profile and strict CPU requirements are unchanged. The underlying
remote ATA timeout is unresolved. Broader ownership/decomposition, interrupt
timing, CPU qualification, independent reference review and later roadmap work
remain open. Roadmap stays **51/100**, Experimental, and PR #9 draft/unmerged.

## October 11 scoped physical memory state and initialization

`fb8600931085209ad1d4f72d96edb098c1136778` replaces separate raw allocator,
grant-ledger and fault-counter access with one permanent in-place `MemoryState`.
Its private `ContextSlot` adapter masks IRQs before publication/access and retires
both guards before restoring caller IF. All observations share that scope.
Higher-ranked callbacks cannot return a state borrow; nested access refuses
before dereference rather than waiting. Initialization requires IF clear and
may start once; ready methods deny access before initialization. Invalid maps
or capacity halt without publishing readiness or promising recoverable rollback.
Serial initialization diagnostics use owned copies outside the state borrow.
Allocation still zeroes before returning a grant; release retains identity/pin,
retirement and scrubbing obligations. No capacity, quota or ABI changed.

Four unsafe blocks were removed; the reviewed inventory now records **429 sites
across 101 sources**, with no new unsafe Send/Sync. The BSP/emergency exclusions
remain mandatory. This does not qualify SMP, complete memory/reference/device
retirement, worst-case masked duration or the wider F4/F5 audit. See
[memory access](MEMORY_ACCESS.md) for caller and failure obligations.

- **324 workspace unit tests**, **two compile-fail doctests**, **130 Python tests**
  and strict kernel host/normal/validation target lint passed. Logs remain at
  `build/memory-access-host-tests.log`, `build/memory-access-python-tests-expanded.log`
  and `build/memory-access-*-clippy.log`. The initial fixture self-test's
  formatting-sensitive preservation assertion failed; it remains separately at
  `build/memory-access-python-tests.log`. The corrected assertion matches the
  formatted allocation call, and the insertion anchor specifically excludes the
  hygiene probe before any live fixture ran. No VM failure was relabeled.
- Pinned strict-provenance Miri passed all **78 selected tests**, and native
  allocator stress passed, on clean `fb86009`. Source/archive, toolchain, commands
  and per-module hashes remain at
  `build/pure-rust-evidence/1791671903010068000/manifest.json`. This interprets the
  reused slot and pure allocator/ledger modules; the privileged adapter/physical
  operations are outside Miri scope. The unchanged bounded ownership-model lane
  was not rerun.
- The complete system suite passed all **eighteen** manifests on clean `fb86009`:
  storage creation/restore/read-only/recovery/failure, serial, network/no-HTTP,
  external SDK, memory preflight/ten allocation cutoffs, six actual protection
  probes and normal debug/release process/file workflows. Source remained
  unchanged throughout the campaign. Paths/hashes are indexed in
  `build/fb86009-system-evidence-index.json`; combined output remains in
  `build/memory-access-system-tests.log`.

Four independent live fixtures passed on clean `fb86009`, each retaining its
manifest under `build/memory-access-evidence/1791671903010051000/`: `before-init`,
`init-repeat`, `reentrant` and `irq-deferred`. The first three require their
distinct production software refusals and no continuation, CPU fault, reset,
stopped VM or exit. The allocation case holds the real state borrow after a grant
is allocated and before production zeroing. QMP sends one real keyboard event;
the guest requires a pending byte, unchanged IRQ count and empty queue until a
serial handshake releases the window. After borrow/publication retirement and
IF restoration, the actual handler delivers exactly one `a`. The returned grant
is live and completely zeroed; poisoning/release scrubs it and restores the frame
baseline. Nested masked memory observations/release preserve IF clear, and the
outer caller retains enabled IF. It covers one delayed allocation/keyboard
schedule, not exhaustive IRQ nesting, physical devices or DMA retirement.

The additional `init-if-enabled` fixture passed on clean
`1cfabc1ec1b40e962636d74c9eeb069ce61d7350`, at
`build/memory-access-evidence/1791672243316747000/init-if-enabled/manifest.json`.
Calling initialization after real IRQ enable requires the context refusal, not
the already-initialized reason. No Rust source changed after `fb86009`.
The shared paused-start/raw-QMP runner gained explicit serial contracts while
retaining its default checks. Both original defaults passed again on clean
`fb86009`: input delivery at
`build/input-irq-evidence/1791671954839429000/manifest.json` and root-observation
refusal at `build/cpu-paging-evidence/1791671954839313000/manifest.json`.

Five independent memory jobs raise the versioned contract and main branch
protection to **69 required app-bound checks**. All other fields were preserved,
with only the order-insensitive context/check arrays compared canonically.
Before/request/response/fresh-after records and hashes remain at
`build/ci-policy-evidence/memory-access-policy-1791672302127687000/`.

The prior published head `3733bbaeebe6877da88f5db86ca56aaf1b228b0a` completed all
**64 then-required checks: 60 passed and four failed**. Its
[main CI](https://github.com/ErzenXz/GenOS/actions/runs/38091711830) passed every
job, including the complete remote system suite. Input IRQ, root-observation,
scoped-borrow and fresh-volume failure passed in
[foundation containment](https://github.com/ErzenXz/GenOS/actions/runs/38091711861).
Strict initial-RIP/NMI-IRET and
[user/kernel #XM](https://github.com/ErzenXz/GenOS/actions/runs/38091711840)
still failed. Artifacts remain under `build/remote-evidence/3733bba/`.
Its exact-head policy/execution audit at
`build/ci-policy-evidence/3733bba-final-audit/` uses the historical 64-check
contract and does not certify green verification. Required-success replay fails
at `build/ci-policy-evidence/3733bba-required-success/`. Both audits predate the
69-check update; the memory change's remote execution must be evaluated separately.

Local QEMU 11.1.2/retained firmware identity remains portability evidence against
the unchanged frozen 11.1.1 profile. No strict CPU requirement was relaxed, and
the remote ATA timeout remains unresolved. Complete ownership/decomposition,
interrupt timing, CPU/reference qualification, independent review and later
roadmap work remain open: **51/100**, Experimental, with PR #9 draft/unmerged.

## October 11 scoped network and storage coordinators

`323ae1d607fe03a6fc674fce4b8b1a9133688c13` replaces raw global protocol access
with an owned `ScopedCell<NetworkStack>` and groups storage scratch space, ATA
port authority and mount phase behind another scoped cell. All global network
reads/mutations and production storage mount/commit paths use higher-ranked
callbacks. ATA PIO helpers receive the borrowed port pair explicitly. The one
new unsafe Sync implementation requires Send values, protects its single value
dereference with acquire/release exclusion, and rejects contenders before value
access. It does not wait, poison, roll back or alter caller IF. Host unwinding
retires the borrow while retaining prior mutations. Seven host/Miri cases and
two new compile-fail checks exercise that contract. The safety rationale and
hardware caller obligations are in [coordinator access](COORDINATOR_ACCESS.md).

Network initialization and storage mount may start once per boot; unsuccessful
discovery consumes that attempt too. Reentry and repeated initialization/mount
halt with distinct production refusal records before another access or I/O.
Network IRQ readiness publication remains available during scopes, and bounded
storage PIO preserves caller IF. One BSP, synchronous coordination, no ordinary
IRQ borrowing of these states and fatal emergency isolation remain mandatory.
The scoped cell does not establish device/DMA retirement or hardware SMP safety.
ABI, image budgets, protocol/storage behavior and frozen reference remain unchanged.
The reviewed lexical inventory records **401 sites across 102 sources**: 29 fewer
unsafe blocks and one additional unsafe impl, for a net reduction of 28 sites.
This scoped review does not close the full F4 unsafe-boundary audit.

- **331 workspace unit tests**, **four compile-fail doctests**, **135 Python tests**
  and strict kernel host/normal/validation target lint passed. Logs remain at
  `build/coordinator-cell-host-tests.log`, `build/coordinator-cell-python-final.log`
  and `build/coordinator-cell-*-clippy.log`.
- Four independent live software fixtures passed on clean `323ae1d`, under
  `build/coordinator-access-evidence/1791673212136649000/`: `network-borrow`,
  `network-repeat`, `storage-borrow` and `storage-repeat`. Each preserves the
  production cell, adapter and fatal reporter and requires exact refusal ordering,
  no continuation/fault, pre-start QMP subscription and a live final VM status.
  These do not inject CPU or device faults. No NIC/data disk is required for the
  unavailable-discovery and ownership-only tests. Archive/fixture/image/log/QMP
  hashes and exact environment remain in individual manifests.
- The production fresh-volume write-failure regression passed on clean `323ae1d`
  at `build/storage-startup-evidence/1791673212136727000/manifest.json`. It exercises
  the new storage scratch/port scope through original quarantine and shell
  behavior; the fixture anchor follows the explicitly passed ATA port authority.
- Pinned strict-provenance Miri passed **85 selected tests**, including all seven
  new scoped-cell cases, and native allocator stress passed on clean `323ae1d`.
  Complete source/archive/toolchain identity, commands, deadlines and per-module
  logs/hashes remain at
  `build/pure-rust-evidence/1791673212136592000/manifest.json`.
  One fixed-seed interpreted execution is not exhaustive concurrency exploration;
  privileged CPU/device behavior remains outside Miri. The unchanged bounded
  ownership-model lane was not rerun.

The complete system suite passed all **eighteen** manifests on clean `323ae1d`:
storage creation/restore/read-only/recovery/failure, serial, network/no-HTTP,
external SDK, memory preflight/ten allocation cutoffs, six actual protection
probes and normal debug/release process/file workflows. Source remained unchanged
throughout the campaign. Paths/hashes are indexed in
`build/323ae1d-system-evidence-index.json`; combined output remains at
`build/coordinator-cell-system-tests.log`. A final local evidence check verified
the source/status and retained hashes for these manifests, the four coordinator
cases, fresh-volume regression and interpreted module logs.

Four independently scheduled coordinator jobs raise the versioned contract and
main protection to **73 required GitHub Actions app-bound checks**. Fresh readback
verifies that all other fields remain unchanged, comparing only the context/check
array ordering canonically. Before/request/response/after records and hashes remain
at `build/ci-policy-evidence/coordinator-access-policy-1791673224430611000/`.

The previous published head `bec495983ab626ed4f503ce55ecbea0201fbcac4` completed
all **69 then-required checks: 65 passed and four failed**. Its
[main CI](https://github.com/ErzenXz/GenOS/actions/runs/38092684786) passed every
job, including the complete remote system suite. All five memory phase/borrow/
allocation-IRQ jobs and the production storage-create-failure regression passed
in [foundation containment](https://github.com/ErzenXz/GenOS/actions/runs/38092684783).
Strict initial-RIP/NMI-IRET and
[user/kernel #XM](https://github.com/ErzenXz/GenOS/actions/runs/38092684860)
still failed. Retained artifacts remain under `build/remote-evidence/bec4959/`,
with paths/hashes indexed in `evidence-index.json`. The historical exact-head
policy/execution audit at `build/ci-policy-evidence/bec4959-final-audit/` records
those 69 executed checks and does not certify green verification. Required-success
replay fails at `build/ci-policy-evidence/bec4959-required-success/`. Both predate
the 73-check upgrade; the new coordinator head needs separate remote execution.

Local QEMU 11.1.2 and retained firmware identity remain portability evidence
against the unchanged frozen 11.1.1 profile. The four strict CPU requirements
remain enforced; the remote ATA timeout's underlying cause remains unresolved.
Complete ownership/decomposition, IRQ timing and nesting, CPU/reference
qualification, independent review, sustained workloads and later roadmap work
remain open. Roadmap stays **51/100**, Experimental; PR #9 remains draft/unmerged.

## October 11 persistent VFS transaction ownership

`ac22cd8fd3b256083e47757164a0aa3e61136e16` replaces the runtime's raw rollback
copies with a permanent owned `VfsSnapshot` in the existing `ScopedCell`.
One borrow covers capture, RAM mutation, commit, any restore and failure-status
publication. All five mutation paths use `mutate_vfs`; no separately callable raw
capture/restore functions remain. Higher-ranked callbacks cannot return either
view; safe array/metadata copies replace two unsafe blocks. Mutation errors restore
the prior view and skip storage. Storage failures still restore before publishing
quarantine status, with no later runtime restore to erase it. No heap allocation,
ABI, VFS limits, media format or device semantics changed. See
[VFS transactions](VFS_TRANSACTIONS.md) for caller, ordering and failure obligations.

The reviewed lexical inventory records **399 sites across 103 sources**, with
no new unsafe construct or Send/Sync implementation. The permanent buffer avoids
another full VFS field in each runtime owner. IRQs remain available; ordinary IRQ
handlers do not access the rollback state, and emergency Rust reentry remains
forbidden. Wider ownership, lock order, DMA, SMP and latency qualification remain
open in F4/F5.

- **336 workspace unit tests**, **five compile-fail doctests**, **135 Python tests**
  and strict kernel host/normal/validation target lint passed. Logs remain at
  `build/vfs-rollback-host-final.log`, `build/vfs-rollback-python-tests.log`,
  `build/vfs-rollback-host-clippy-final.log`, `build/vfs-rollback-target-clippy.log`
  and `build/vfs-rollback-validation-clippy.log`.
- Five pure snapshot tests cover capture/mutation order, complete full-capacity
  restore, latest acknowledged state, independent owners and post-restore status.
  The existing production storage fault matrix now routes all **176** before/after
  write/flush failures through the actual snapshot and safe restore callback,
  retaining prior RAM/media/quarantine/no-I/O assertions and adding exact
  not-published/unknown status checks. This is the existing bounded simulated disk
  matrix, not physical-device fault qualification.
- The live `vfs-borrow` software fixture passed on clean `ac22cd8`, at
  `build/coordinator-access-evidence/1791673824237696000/vfs-borrow/manifest.json`.
  It holds a real snapshot callback while reentering the production rollback
  adapter and requires `VFS_TRANSACTION_REJECTED policy=halt` without continuation,
  fault, reset, VM exit or stopped status. The original paused-start/raw-QMP runner
  and exact serial contract are reused. It does not inject a CPU or device fault.
- Pinned strict-provenance Miri passed all **90 selected tests**, including the
  five new snapshot cases, and native allocator stress passed on clean `ac22cd8`.
  Complete source/archive/toolchain, commands, deadlines and module log hashes
  remain at `build/pure-rust-evidence/1791673824237644000/manifest.json`.
  Privileged CPU/device operations remain outside the interpreter scope; one
  fixed seed is not exhaustive concurrency exploration. The unchanged bounded
  ownership-model lane was not rerun.
- The complete system suite passed all **eighteen** manifests on clean `ac22cd8`:
  storage creation/restore/read-only/recovery/failure, serial, network/no-HTTP,
  external SDK, memory preflight/ten allocation cutoffs, six actual protection
  probes and normal debug/release process/file workflows. Source remained clean
  and unchanged throughout. Paths/hashes are indexed in
  `build/ac22cd8-system-evidence-index.json`; output remains at
  `build/vfs-rollback-system-tests.log`. A final check verified source/status and
  hashes for these manifests, the live VFS fixture and all interpreted logs.

The independent VFS transaction lane raises the versioned contract and main
protection to **74 required GitHub Actions app-bound checks**. Fresh readback
compares every other field exactly, normalizing only context/check array order.
Before/request/response/after records and hashes remain at
`build/ci-policy-evidence/vfs-rollback-policy-1791674128862651000/`.

The previous published head `3f6ba38dfc381aaebe8c3a29e934f6836fcad3f7` completed
all **73 then-required checks: 69 passed and four failed**. Its
[main CI](https://github.com/ErzenXz/GenOS/actions/runs/38093785138) passed every
job, including the complete remote system suite. All four coordinator refusal
jobs and the production storage-create-failure regression passed in
[foundation containment](https://github.com/ErzenXz/GenOS/actions/runs/38093785145).
Strict initial-RIP/NMI-IRET and
[user/kernel #XM](https://github.com/ErzenXz/GenOS/actions/runs/38093785153)
still failed. Retained artifacts remain under `build/remote-evidence/3f6ba38/`,
with paths/hashes indexed in `evidence-index.json`. The historical exact-head
policy/execution audit at `build/ci-policy-evidence/3f6ba38-final-audit/` records
73 completed checks and does not certify green verification. Required-success
replay fails at `build/ci-policy-evidence/3f6ba38-required-success/`. Both predate
the 74-check upgrade; the VFS change needs separate remote execution.

Local QEMU 11.1.2 and retained firmware identity remain portability evidence
against the unchanged frozen 11.1.1 profile. The four strict CPU requirements
remain enforced, and the remote ATA timeout's underlying cause is unresolved.
Complete ownership/decomposition, IRQ timing/nesting, CPU/reference qualification,
independent review, sustained workloads and later roadmap work remain open.
Roadmap stays **51/100**, Experimental; PR #9 remains draft/unmerged.

## October 11 dormant framebuffer ownership and raw mapping boundary

`33810c08dcb43997afa9b7061481bda2d9c2a396` includes the framebuffer ownership
change from `c121a598706ac0c60cc5890f98446b482b844803` and its raster correction.
The implementation removes the dormant renderer's safe
arbitrary-address constructor and shared mutable global backbuffer. A safe
RAM-backed constructor now borrows exclusive caller-owned target/backbuffer
slices. The separate unsafe MMIO constructor records mapping, alignment,
non-trapping stores, external-memory/device exclusivity and lifetime obligations;
there is currently no production caller. Numeric admission rejects invalid
layout, format, extent, alignment, short buffers and overlapping backbuffer
ranges before constructing a device. No Rust MMIO reference is fabricated.
DisplayManager now carries the borrowed storage lifetime, and renderer/recovery
functions borrow that typed owner. The serial composition root still creates no
framebuffer; its source guard now rejects all direct FramebufferDevice calls.
ABI, limits and media/reference contracts are unchanged.

Raster entry uses wide clipping/index arithmetic for extreme rectangles,
blits, lines and circles; RAM and MMIO writes have distinct bounds/provenance
obligations. RGB/BGR packing retains the zero reserved byte, and presentation
preserves padding/tail storage. The reviewed inventory now records **398 sites
across 103 sources**, one fewer unsafe block. Three framebuffer unsafe constructs
have explicit obligations in [the ownership review](FRAMEBUFFER_OWNERSHIP.md).
No unsafe Send/Sync implementation was added; the device is neither Send nor
Sync. This is a scoped safety/interface fix, not graphical hardware qualification
or completion of F4/F5/Stage 9.

- **347 workspace unit tests**, **nine compile-fail doctests**, **135 Python tests**
  and strict kernel host/normal/validation lint passed. Logs remain at
  `build/framebuffer-phase-host-tests.log`,
  `build/framebuffer-phase-python-tests.log` and
  `build/framebuffer-phase-*-clippy.log`. Extracting the numeric alias helper
  initially left a double dereference after changing its parameter to a slice;
  the compiler rejected it. Those failed compiler/Python-wiring runs remain at
  `build/framebuffer-alias-extraction-*.log`; the corrected helper uses the slice
  directly and the final checks above passed. No failed VM/Miri run was relabeled.
- Eleven new tests cover rejected dimensions/formats/extents/buffers, numeric MMIO
  range/overlap admission without access, RGB/BGR packing, buffered present and
  overlay restore, extreme geometry/blits/lines/circles, padding/tail canaries,
  independent surfaces and small-surface wallpaper/checker/stroke/overlay work.
  Four new compile-fail cases reject buffer aliasing, escaping owner lifetimes,
  safe MMIO construction and thread transfer.
  The eleventh compares all 4,096 ordered endpoint pairs in a bounded grid
  against an independent iterative Bresenham oracle and checks padding canaries.
- The initial clipping implementation passed its 100 selected Miri tests and
  all 18 system workflows on clean `c121a59`, but a subsequent semantic oracle
  found that clipping endpoints changed raster phase: the line from (-2,-2) to
  (0,2) lost pixel (0,1). This was a visible-output error, not a Miri finding.
  The counterexample remains at `build/framebuffer-line-phase-counterexample.log`;
  initial evidence remains at
  `build/pure-rust-evidence/1791674880213158000/manifest.json` and
  `build/c121a59-system-evidence-index.json`. The corrected path derives the
  original Bresenham phase with wide arithmetic and skips invisible major-axis
  steps. The retained oracle and extreme/even-width brush tests pass.
- Pinned `nightly-2026-09-21` Miri with strict provenance and seed 1 passed
  **101 production-selected tests** on clean `33810c0`, plus the unchanged native
  20,000-operation allocator stress. Manifest, per-module logs and hashes remain
  at `build/pure-rust-evidence/1791675362050061000/`. This includes all eleven
  framebuffer tests on borrowed RAM and numeric MMIO admission without touching
  device memory. Miri does not execute real MMIO, IRQ, DMA or page-table hardware.
- The complete `cargo xtask test` suite passed **18 clean-source system workflows**
  on `33810c0`: storage creation/restore/read-only/recovery/failure, serial input,
  network/no-HTTP, external SDK, memory preflight/ten allocation cutoffs, six
  actual protection probes and normal debug/release process/file workflows.
  Paths/hashes are indexed in `build/33810c0-system-evidence-index.json`; output
  remains at `build/framebuffer-phase-system-tests.log`. A final check verified
  source/status and hashes for all manifests and interpreted/native logs before
  this documentation update. Local QEMU 11.1.2 remains portability evidence
  against the unchanged frozen 11.1.1 profile.

The prior published head `a77406d693ce86de8753d24f5d14d5e1a7e573a8` completed
all **74 required checks: 70 passed and four failed**. Its
[main CI](https://github.com/ErzenXz/GenOS/actions/runs/38094451756) passed the
complete remote system suite. All five coordinator refusal jobs, including VFS
transaction reentry, and production storage-create-failure passed in
[foundation containment](https://github.com/ErzenXz/GenOS/actions/runs/38094451762).
Strict initial-RIP/NMI-IRET and
[user/kernel #XM](https://github.com/ErzenXz/GenOS/actions/runs/38094451842)
still failed. Artifacts remain under `build/remote-evidence/a77406d/`, with
29 retained manifests (including workflow duplication), paths and hashes indexed
in `evidence-index.json`. The exact-head policy/execution audit at
`build/ci-policy-evidence/a77406d-final-audit/` passed its 74-check contract audit
and explicitly records failed verification. Required-success replay fails at
`build/ci-policy-evidence/a77406d-required-success/`. Main protection remains
unchanged at 74 strict app-bound checks; no CPU requirement was removed or relaxed.
The new framebuffer head requires its own remote execution.

Complete ownership/decomposition, IRQ timing/nesting, CPU/reference qualification,
independent review, sustained workloads, actual graphics and later platform work
remain open. The underlying remote ATA timeout remains unresolved. Roadmap stays
**51/100**, Experimental; PR #9 remains draft/unmerged.

## October 11 VirtIO TX timeout and completion ownership

`899624252d3269e0af0a0d99d42f7c5187297c59` fixes reuse of the single permanent
TX DMA buffer after an uncertain publication. The old driver returned false on
its polling deadline, then let the next send rewrite the same buffer/descriptor
without observing completion. It also accepted any changed used index. A new
production `TxQueue` tracks one in-flight descriptor, requires exactly one
wrapping used-index advance with ID zero, and makes timeout/malformed completion
quarantine irreversible for that instance. Reservation precedes every TX buffer
write. Further TX/RX calls return before mutation or notification; even a late
valid completion cannot authorize reuse. Rejected completions leave the owned
used counter and successful-TX metrics unchanged. Healthy queue layout, capacity,
interrupt/coordinator ordering, ABI, limits and frozen reference remain unchanged.

The permanent buffers stay reserved: quarantine does not stop DMA, establish
reset completion, scrub/free memory or authorize another initialization. No
recovery/reset path is added. The [ownership review](TX_QUEUE_OWNERSHIP.md) records
caller obligations and remaining raw-DMA, RX, mapping, reset and isolation work.
This scoped fix does not close F3.2/F4/F6.4/H-DMA or qualify physical devices.

- An archived-source host fixture reproduces the old overwrite on published
  `719757d7763d373367e45719682950dbb0e1d358`: after two real production polling
  timeouts the first packet byte changes from 0x45 to 0x99. The failing assertion,
  exact patch, command, source/archive identity and hashes remain at
  `build/tx-timeout-counterexample/`. Notification targets borrowed RAM and no
  device DMA runs in this host counterexample; it is not a hardware fault claim.
- **353 workspace unit tests**, **nine compile-fail doctests**, **135 Python tests**
  and strict host/normal/validation kernel lint passed. Logs remain at
  `build/tx-ownership-host-tests.log`, `build/tx-ownership-python-tests.log` and
  `build/tx-ownership-*-clippy.log`. The reviewed inventory records **400 lexical
  sites across 104 sources**: two added unsafe blocks are in the existing
  exclusive host DMA fixture; no production unsafe block or unsafe Send/Sync
  implementation was added. Initial inventory-drift checks failed as expected
  until explicit source/context review and baseline replacement.
- Six new pure tests cover success, busy reservation, timeout/late completion,
  bad descriptor IDs, stale/skipped/excess used indices, unsolicited/duplicate
  completion and wrap. The existing single driver fixture retains its coalesced
  RX/wrap/invalid-advance checks and now executes production TX success, bad ID,
  excess advance and the real polling timeout. After each rejected completion it
  injects a plausible late completion and checks denied send/receive, unchanged
  buffer bytes, descriptor fields, available index and notification count.
- Pinned `nightly-2026-09-21` Miri with strict provenance and seed 1 passed
  **107 selected tests**, including all six new TX-state tests, plus the unchanged
  native 20,000-operation allocator stress, on clean `8996242`. Manifest,
  per-module logs and hashes remain under
  `build/pure-rust-evidence/1791675963142701000/`. Real DMA, MMIO/PIO, interrupts
  and CPU/page-table semantics remain outside interpreter execution.
- The complete `cargo xtask test` suite passed **18 clean-source system workflows**
  on `8996242`, including both real modern VirtIO network/no-HTTP workflows,
  storage creation/restore/read-only/recovery/failure, serial input, external SDK,
  memory preflight/ten allocation cutoffs, six actual CPU protection probes and
  normal debug/release process/file workflows. Paths/hashes remain in
  `build/8996242-system-evidence-index.json`, with output at
  `build/tx-ownership-system-tests.log`. Final qualification checked clean source,
  manifest status/identity and interpreted-log hashes before this ledger update.
  Healthy transport evidence does not inject a real device timeout or prove DMA
  quiescence. Local QEMU 11.1.2 remains portability evidence against frozen 11.1.1.

The previous framebuffer head `719757d7763d373367e45719682950dbb0e1d358`
completed all **74 required checks: 70 passed and four failed**. Its
[main CI](https://github.com/ErzenXz/GenOS/actions/runs/38095871073) passed the
complete remote system suite. All five coordinator refusal cases and production
storage-create-failure passed in
[foundation containment](https://github.com/ErzenXz/GenOS/actions/runs/38095871096).
Strict initial-RIP/NMI-IRET and
[user/kernel #XM](https://github.com/ErzenXz/GenOS/actions/runs/38095871100)
still failed. Retained artifacts, paths and hashes remain at
`build/remote-evidence/719757d/`; its index covers 29 manifests with workflow
duplication, not 29 distinct required checks. The exact-head policy/execution
read at `build/ci-policy-evidence/719757d-final-audit/` passed its contract audit
while explicitly recording failed verification. Required-success replay fails
at `build/ci-policy-evidence/719757d-required-success/`. Main protection remains
unchanged at 74 strict app-bound checks; all four CPU requirements stay enforced.
The new TX head requires its own remote execution.

Roadmap stays **51/100**, Experimental; PR #9 remains draft/unmerged. Broader
ownership, IRQ latency/nesting, CPU/reference qualification, independent review,
sustained workloads and later platform/application work remain open. The
underlying remote ATA timeout remains unresolved.

## October 11 immutable VirtIO RX completion batches

`a986189a89da67849f670db0866ef945da905411` moves RX used-counter/batch ownership
into the production `RxBatch<8>`. Before copying any packet or reposting any
buffer, the driver captures and validates the whole bounded used-ring prefix:
exact wrapping advance/count, every descriptor ID, uniqueness within that
snapshot and every raw DMA extent. It applies acquire ordering after observing
the device's used index and keeps the admitted records immutable while draining.
New interrupt readiness remains pending for the next snapshot. Later snapshots
may legitimately complete a descriptor that the coordinator copied/reposted
from an earlier batch. Packet lengths above the unchanged 1518-byte frame budget
but inside the valid DMA buffer retain the existing drop/repost policy.

Duplicate, invalid-ID or invalid-extent batches quarantine before the first
copy/repost, leave the caller's packet and owned used counter unchanged, and
publish no success or notification. Later RX/TX entry remains denied even after
a plausible late record. Quarantine does not establish reset/quiescence or free,
scrub or reconstruct DMA storage. The [RX ownership review](RX_QUEUE_OWNERSHIP.md)
records caller obligations, coherent/trusted-device assumptions and remaining
raw-DMA, claim/initialization, mapping/reset/isolation work. ABI, permanent DMA
capacity, network frame budget and the frozen reference remain unchanged; this
does not close F3.2/F4/F6.4/H-DMA or qualify malicious/physical devices.

- An archived-source counterexample on prior published
  `6d149ad73b6602d3f935744dd40e641687777ee7` captures two used entries with the
  same descriptor ID. The old production driver accepts the first packet and
  advances available.index from eight to nine, returning that buffer before
  validating the duplicate. The failing assertion, patch, source/archive
  identity, command and hashes remain at `build/rx-duplicate-counterexample/`.
  Notifications target borrowed RAM; no hardware DMA runs in that host fixture.
- **360 workspace unit tests**, **nine compile-fail doctests**, **135 Python tests**
  and strict kernel host/normal/validation lint passed. Logs remain at
  `build/rx-batch-final-host-tests.log`, `build/rx-batch-final-python-tests.log`
  and `build/rx-batch-final-*-clippy.log`. The seven new pure tests cover ordered
  drain, all 28 duplicate-position pairs in a full batch, malformed extents/IDs
  after a valid prefix, stale/excess/incomplete snapshots, pending-batch
  replacement, legitimate later reuse, empty/wrapping indices, quarantine and
  separate packet-size admission. Existing TX regressions remain intact.
- Follow-up `c6757356a10cbfc65f0b941b1f84652b031415f0` keeps the original
  one-interrupt/two-packet coalescing check separate from the new arrival/IRQ
  during a pending batch. Its complete **360 unit/nine compile-fail** run passed
  at `build/rx-batch-isolated-host-tests.log`. The diff changes only the driver's
  `cfg(test)` fixture and its inventory. The production prefix is byte-identical
  to `a986189`; all other sources and Miri selections are unchanged. Exact
  correspondence and the host-log hash remain at
  `build/rx-batch-production-correspondence.json`. The reviewed inventory now
  records **404 lexical sites across 105 sources**, four added host-fixture
  unsafe blocks across these commits and no added production unsafe block or
  unsafe Send/Sync implementation. Inventory-drift checks failed as expected
  before explicit source/context review and baseline replacement.
- Pinned `nightly-2026-09-21` strict-provenance Miri with seed 1 passed **114
  selected tests**, including all seven RX-state tests, plus the unchanged native
  20,000-operation allocator stress, on clean `a986189`. Manifest, per-module
  logs and hashes remain at `build/pure-rust-evidence/1791676687144625000/`.
  Actual DMA, MMIO/PIO, interrupt and CPU/page-table semantics remain excluded.
- The complete `cargo xtask test` suite passed **18 clean-source workflows** on
  `a986189`, including modern VirtIO network/no-HTTP, storage
  creation/restore/read-only/recovery/failure, serial input, external SDK,
  memory preflight/ten allocation cutoffs, six CPU protection probes and normal
  debug/release process/file workflows. Paths/hashes are indexed at
  `build/a986189-system-evidence-index.json`; output remains at
  `build/rx-batch-system-tests.log`. Final checks verified clean source,
  manifest/log identity and hashes, the archived counterexample and the host-only
  follow-up correspondence before this documentation update. Healthy transport
  tests do not inject a real malformed device batch or establish DMA isolation.
  Local QEMU 11.1.2 remains portability evidence against frozen 11.1.1.

The previous TX head `6d149ad73b6602d3f935744dd40e641687777ee7` completed all
**74 required checks: 70 passed and four failed**. Its
[main CI](https://github.com/ErzenXz/GenOS/actions/runs/38096425211) passed the
complete remote system suite. All five coordinator refusal cases and production
storage-create-failure passed in
[foundation containment](https://github.com/ErzenXz/GenOS/actions/runs/38096425216).
Strict initial-RIP/NMI-IRET and
[user/kernel #XM](https://github.com/ErzenXz/GenOS/actions/runs/38096425274)
still failed. Retained artifacts and indexed paths/hashes remain at
`build/remote-evidence/6d149ad/`; 29 manifests include workflow duplication rather
than 29 distinct required checks. The exact-head policy/execution read at
`build/ci-policy-evidence/6d149ad-final-audit/` passed its contract audit while
explicitly recording failed verification. Required-success replay fails at
`build/ci-policy-evidence/6d149ad-required-success/`. Main protection remains
unchanged at 74 strict app-bound checks; all four CPU requirements stay enforced.
The new RX head requires its own remote execution.

Roadmap stays **51/100**, Experimental; PR #9 remains draft/unmerged. Broader
ownership, IRQ latency/nesting, CPU/reference qualification, independent review,
sustained workloads and later platform/application work remain open. The
underlying remote ATA timeout remains unresolved.

## October 11 permanent VirtIO allocation claim

`321813eaecb83442325b3cde22cf54c8245592b1` replaces four raw mutable DMA globals
with one static `PermanentDma<VirtioDmaMemory>`. Discovery claims before any
PCI/PIO/MMIO access and moves its unique token into the driver. Failed or absent
device initialization, fallback, drop and unwind never release that claim.
Repeated discovery or owner initialization halts before hardware I/O with
`NETWORK_DEVICE_INIT_REJECTED policy=halt`. Existing reset-before-queue-clear,
TX quarantine and immutable RX batch policies remain intact. The allocation
payload remains 28,672 bytes; the aligned wrapper reserves 32,768 bytes, adding
one 4096-byte metadata/alignment page. ABI 18, frame/descriptor budgets and the
frozen reference remain unchanged. [Caller obligations](PERMANENT_DMA.md) retain
mapping/coherence, device directions, completion/reset, removal, isolation and
SMP as separate requirements; this is no claim of DMA quiescence or release.

- The archived old driver on `d203b4322a1d7a5440eff89aa6e6c5c6972ca4f4` returns
  from a second direct discovery after its unavailable first attempt. The
  no-NIC live VM counterexample, source/archive/image identity, fixture patch,
  QMP transcript and hashes remain at `build/permanent-dma-old-counterexample/`.
  The new strict refusal validator rejects that old continuation.
- **366 workspace unit tests**, **13 compile-fail doctests**, **135 Python tests**
  and strict kernel host/normal/validation Clippy passed. The initial workspace
  run failed an obsolete source-wiring assertion for a removed raw global; the
  corrected assertion requires the permanent allocation and owned token. Both
  logs remain at `build/permanent-dma-host-tests.log` and
  `build/permanent-dma-host-tests-rerun.log`; Python/lint logs use the same prefix.
- Pinned `nightly-2026-09-21` Miri passed **120 selected tests** with strict
  provenance/seed 1, including six permanent-claim cases. The unchanged native
  20,000-operation allocator stress and bounded model also passed: **3,257,436
  sequences / 19,248,492 transitions**, with retained injected-oracle failure
  and safe production replay. Exact clean source/archive identity and per-command
  log hashes remain at `build/pure-rust-evidence/1791677804558770000/`.
  Four new compile-fail checks deny movable storage, token cloning/sharing and
  sharing a non-Send payload. One interpreted thread schedule is not exhaustive
  concurrency qualification, and no actual device DMA executes under Miri.
- All **six coordinator refusal VM cases** passed on clean `321813e` under
  `build/coordinator-access-evidence/1791677804558805000/`. The new
  `network-device-repeat` case deliberately calls `stack.device.discover()`
  under the ordinary coordinator borrow, independently of its outer init guard.
  It requires distinct refusal and live QMP status, rejecting continued dispatch,
  CPU faults, reset and stopped/exited guest substitutes. No NIC is attached.
- The complete `cargo xtask test` passed **18 clean-source workflows** on
  `321813e`, including healthy VirtIO/no-HTTP, production storage
  create/restore/read-only/recovery/failure, SDK, serial input, memory pressure
  and ten allocation cutoffs, six CPU protection probes and normal debug/release
  workflows. Manifest identities/hashes are indexed at
  `build/321813e-system-evidence-index.json`; output remains at
  `build/permanent-dma-system-tests.log`. The final audit at
  `build/permanent-dma-verification.json` checks the manifests, all pure-run log
  hashes, coordinator evidence, old counterexample and policy records. Local
  QEMU 11.1.2 remains portability evidence against frozen 11.1.1.
- Explicit source/context review records **409 lexical unsafe/assembly sites
  across 106 sources**. The delta is one production unsafe Sync implementation,
  bounded by Send and the irreversible private claim, plus four pure-test raw
  pointer blocks. No unsafe Send implementation was added. Existing driver raw
  accesses now derive from its token; mapping, bounds, ordering and actual
  CPU/device access obligations still apply. Inventory presence is no whole-kernel
  unsafe review or certification.

Previous published `d203b43` completed **74 required checks: 70 passed, four
failed**. [Main CI](https://github.com/ErzenXz/GenOS/actions/runs/38097246727)
passed; [foundation containment](https://github.com/ErzenXz/GenOS/actions/runs/38097246729)
and [exception entry](https://github.com/ErzenXz/GenOS/actions/runs/38097246760)
retain only the strict initial-RIP/NMI-IRET and user/kernel #XM failures.
All artifacts are retained at `build/remote-evidence/d203b43/`; its integrity audit
checked 92 manifests and 129 retained artifact hashes. Manifest duplication across
workflows is not a required-check count. The read-only audit at
`build/ci-policy-evidence/d203b43-final-audit/` uses that head's historical
74-check contract, records failed verification and retains required-success
rejection. Distribution QEMU/firmware remains portability evidence.

Main protection is now **75 strict app-bound checks**, adding only
`Foundation coordinator-network-device-repeat`. Fresh before/request/response/
after and contract hashes remain at
`build/ci-policy-evidence/permanent-dma-policy-1791677861428065000/`. Readback
compares every unrelated field exactly, normalizing only required-array ordering.
All four strict CPU checks remain enforced. The new published head requires
separate exact-head remote execution.

The original console checklist remains **51/100**, Experimental. PR #9 stays
draft/unmerged. Full roadmap work includes ownership, CPU/reference qualification,
independent review, sustained workloads and later platform/application stages;
this change closes none of their broader gates. The underlying remote ATA timeout
remains unresolved.

## October 11 single-boot lifecycle churn and syscall diagnostic overflow

Production source `2e537dcf57934744c25c60537107c8e04cbe813e` fixes a valid
syscall panicking after its diagnostic byte counter overflows. Ping, ABI, report
and output now use the pure `DiagnosticCounter`: 64-bit atomic event totals and
returned values saturate without wrapping. Initial boot validation still
requires exact totals, so extra calls are not hidden by saturation at a boot
threshold. No allocation/handle authority, ABI 18, capacity budget, CPU entry
policy or frozen reference changed. The four counters occupy 32 rather than
four bytes before surrounding linker alignment; no unsafe code was introduced.

The new [single-boot lifecycle workload](LIFECYCLE_QUALIFICATION.md) only adds a
probe call and function to an immutable source archive. All original process
construction, Ring 3 execution, syscall, context, scrub, handle cleanup and reap
functions remain unchanged. It alternates normal exit and output-observed
kill/reap, reuses one manager's slot with increasing incarnation, and requires
exact live-frame/slot/active baselines, cumulative reclamation, consistent
allocator/ledger state and restored IF after every cycle. The host parser also
requires ordered real production image-load, spawn, user output, terminal,
reclamation and reap records per cycle. The fixed VM budget is 1800 seconds,
separate from the 600-second build budget.

- The first full run on clean `19f53ad248e5ff9f10fc967b794250e16f71ba1a`
  completed 231 cycles and panicked at cycle 232, source line 6503, in the
  production `PING_COUNT.fetch_add(...) + 1`. No fault was injected. Raw logs,
  exact source/archive/image identity and hashes remain at
  `build/lifecycle-churn-evidence/1791678519435789000/`; the counterexample audit
  is `build/lifecycle-counterexample-audit.json`. The initial 20-cycle pass on
  that same source is explicitly exploratory, not a 10,000-cycle qualification.
- The first corrected-kernel 10,000-cycle run and 512-cycle counterpart both
  reached their final VM summaries, then failed host admission because the
  initial parser wrongly excluded PIDs 1–3. Production admits every nonzero
  byte PID. Both failed manifests remain unchanged under runs
  `1791678682128761000` and `1791679092228681000`; they are not relabeled as
  passes. The corrected harness checks the entire 1–255 succession and requires
  PID reuse only after 255 cycles. Fresh runs below provide passing admission.
- Clean harness source `9a9913a5dd657df90bd8d3d3063893b6df83b7f6` passed
  **10,000 cycles: 5,000 normal exits, 5,000 kills, 100,000 reclaimed process
  frames**, with all 255 PIDs reused and the live-frame baseline **1263** after
  every cycle. Source/fixture/image and raw serial/QEMU/QMP evidence remain at
  `build/lifecycle-churn-evidence/1791679263430032000/`. A separate **512-cycle**
  run on the same source passed at `build/lifecycle-churn-evidence/1791679263430055000/`,
  with 256 exits, 256 kills and 5120 reclaimed frames. It explicitly does not
  satisfy the 10,000-cycle count. These totals are cumulative page lifetimes,
  not simultaneous pressure or physical-remanence erasure guarantees.
- **369 workspace unit tests**, **13 compile-fail doctests**, **148 Python tests**
  and strict kernel host/normal/validation Clippy passed. Three new pure tests
  cross the former byte limit, verify u64 saturation and concurrent event totals.
  Logs use `build/lifecycle-counter-*-tests.log`,
  `build/lifecycle-counter-*-clippy.log` and
  `build/lifecycle-delivery-final-python-tests.log`. Inventory source/context
  review retains **409 lexical sites across 107 sources**, with no new unsafe
  block, unsafe Send/Sync or assembly operation.
- Pinned strict-provenance Miri/seed 1 passed **123 selected tests** plus the
  unchanged native allocator stress on clean `2e537dc`; the bounded model again
  passed 3,257,436 sequences and 19,248,492 transitions with its injected-oracle
  counterexample and safe replay. Exact per-command results/hashes remain at
  `build/pure-rust-evidence/1791678682128786000/`. The complete system suite also
  passed **18 clean-source workflows** on that core, including network/no-HTTP,
  storage failures/recovery, SDK, memory/ten allocation cutoffs, six protection
  probes and normal debug/release boots. Its manifest/log hashes are indexed at
  `build/2e537dc-system-evidence-index.json`; output remains at
  `build/lifecycle-counter-system-tests.log`.
- `tools/audit_lifecycle_churn.py` checks source/archive identity, the archived
  literal fixture template without executing archived Python, artifact hashes,
  every lifecycle record, raw QMP correspondence and live status. It rejects
  failed manifests and cannot admit a short run as 10,000 cycles. Replay results
  remain at `build/lifecycle-10000-replay.json` and
  `build/lifecycle-boundary-replay.json`. Final combined checks at
  `build/lifecycle-delivery-verification.json` also prove all compiled kernel,
  bootloader, userspace, ABI, compiler/configuration and Miri/model inputs remain
  unchanged from `2e537dc`; subsequent changes affect harness/audit/CI/docs only.

Previous published `226f8bb3e9bc4b89a00ed560c86dcb87432147f4` completed **75
required checks: 71 passed and four failed**. Its
[main CI](https://github.com/ErzenXz/GenOS/actions/runs/38098174888) passed;
[foundation containment](https://github.com/ErzenXz/GenOS/actions/runs/38098174866)
passed direct-device rediscovery refusal and retains only initial-RIP/NMI-IRET
failures; [exception entry](https://github.com/ErzenXz/GenOS/actions/runs/38098174903)
retains user/kernel #XM failures. All artifacts remain under
`build/remote-evidence/226f8bb/`; integrity checks cover 93 manifests and 137
artifact hashes, including workflow duplication. The early policy read correctly
failed while Build and boot was pending at `build/ci-policy-evidence/226f8bb-final-audit/`.
The completed 75-check audit is `build/ci-policy-evidence/226f8bb-completed-audit/`;
required-success rejection remains at `build/ci-policy-evidence/226f8bb-required-success/`.
Explicit historical-contract replay is retained separately; that option requires
both recorded responses and cannot override live enforcement.

Main now requires **76 strict app-bound checks**, adding only
`Foundation lifecycle-counter-boundary` with 512 cycles and a 180-second VM
budget. Every other policy field is preserved in
`build/ci-policy-evidence/lifecycle-counter-policy-1791679362567210000/`.
The weekly/manual long-validation workflow runs the full fixed 10,000-cycle
command and retains first-failure evidence. Configuration is not remote execution
of that long run, and the new published head needs its own 76-check execution.

F6.6 remains open: this serial one-child workload is a debug validation kernel
with optimized userspace and does not establish 24-hour mixed memory/IPC/I/O
pressure, sustained release behavior, concurrent-child/cancellation/device
schedules or C5's later 72-hour console workload. Local QEMU 11.1.2 remains
portability evidence against frozen 11.1.1. Full roadmap scope, the original
**51/100** console count, Experimental status and draft/unmerged PR #9 remain
unchanged; independent review and all four strict CPU failures stay open.


## October 11 interrupt startup admission and descriptor publication

The initializer formerly rewrote the live IDT and reprogrammed PIC/PIT on every
safe call. The new repeat-before-seal fixture on clean `9884f31` reproduced two
`IRQ_READY` lines and `INTERRUPT_ADMISSION_RETURNED`. Its original failed manifest,
archive/fixture/image identities and raw evidence remain at
`build/interrupt-admission-evidence/1791680141130631000/repeat-before-seal/`.
No injected exception or validator waiver turns that invalid continuation into
passing qualification.

Qualified core **`4f9f4f7d2c0f4a1b46c746eba0a765e2a3f1a48d`** fills all IDT vectors,
including IRQ/MSI-X/syscall overrides and syscall DPL, in the original guarded
architecture pass before LIDT. The later setters are removed. An irreversible BSP
phase gate publishes completed architecture setup, claims PIC/PIT setup once and
admits interrupt enablement only after controller readiness. Rejected calls clear
IF and halt before controller I/O or STI. Main still seals IDT/kernel mappings
before enabling IRQs. [The contract](INTERRUPT_ADMISSION.md) records remaining
unsafe/raw CPL0, TSS/emergency, hardware and SMP obligations.

- All **four new VM cases** passed on that clean source: setup before architecture,
  repeat before sealing, repeat after sealing with IRQs enabled, and enablement
  before controller setup. Evidence is at
  `build/interrupt-admission-evidence/1791680251579028000/`. Call-only fixtures
  preserve production checks/hardware code and require exact ordered refusal,
  no CPU exception or continued boot, and running QMP status after deliberate
  halt. Raw QMP/serial/build/command files and hashes remain retained.
- All **five original BSP fixtures** passed, including one/four-vCPU normal boot,
  repeated entry, repeated architecture initialization and injected non-BSP role.
  Evidence is at `build/bsp-evidence/1791680301554812000/`; the injected role case
  does not simulate an AP trampoline.
- **371 workspace unit tests**, **13 compile-fail doctests**, **154 Python tests**
  and strict kernel host/normal/validation Clippy passed. Logs use
  `build/interrupt-phase-workspace-tests.log`, `build/interrupt-phase-python-tests.log`
  and `build/interrupt-phase-*-clippy.log`. The inventory now records **405 lexical
  sites across 107 sources**, removing four lexical unsafe-function sites from
  the two setters and their unsafe pointer signatures. No new unsafe block,
  unsafe Send/Sync or assembly is added; the complete caller audit remains open.
- Pinned strict-provenance Miri/seed 1 passed **131 selected tests**, adding all
  eight boot/table/controller identity, ordering/readiness and competing-claim
  tests. Native 20,000-operation allocator stress and the bounded model's
  3,257,436 sequences / 19,248,492 transitions passed again, including retained
  injected-oracle and safe replay records. Exact commands/results/log hashes
  remain at `build/pure-rust-evidence/1791680252680995000/`.
- The full suite passed **18 system workflows** on this clean core, including
  network/no-HTTP, storage creation/restore/read-only/failure/recovery, SDK,
  serial input, memory/ten allocation cutoffs, six CPU protection probes, and
  normal debug/release boots with real process/file I/O. The manifest index is
  `build/4f9f4f7-system-evidence-index.json`; the command log is
  `build/interrupt-phase-system-tests.log`.
- A fresh **512-cycle** Ring 3 run passed on the new core with 256 exits, 256 kills,
  5120 reclaimed frames, all 255 nonzero PIDs and exact baseline 1263 after
  every cycle. Evidence is at `build/lifecycle-churn-evidence/1791680372534999000/`;
  archived-source/fixture/raw-QMP replay passed at
  `build/interrupt-phase-lifecycle-replay.json`. This short run explicitly does
  not qualify 10,000 cycles. The earlier full-count result retains its earlier
  core/harness identity and is not reattributed to this changed architecture.

Combined source/archive, fixture reconstruction, serial/QMP, manifest/log and
policy checks remain at `build/interrupt-delivery-verification.json`. The core
was committed clean before execution. Subsequent publication changes are Markdown
only; correspondence records that fact instead of claiming tests executed on a
later documentation SHA. These self-consistency checks are not signed attestation
or independent review.

Previous published **`ae586e1d53cd6016253546e97a7097347c402eb6`** completed **76
required checks: 72 passed and four failed**. [Main CI](https://github.com/ErzenXz/GenOS/actions/runs/38099810766)
passed; [Foundation](https://github.com/ErzenXz/GenOS/actions/runs/38099810756)
passed the new 512-cycle regression and retains initial-RIP/NMI-IRET failures; [Exception entry](https://github.com/ErzenXz/GenOS/actions/runs/38099810773)
retains user/kernel #XM failures. All workflow artifacts remain under
`build/remote-evidence/ae586e1/`; integrity checks cover **94 manifests and 145
artifact hashes**, including duplication. The completed 76-check reporting audit
is `build/ci-policy-evidence/ae586e1-completed-audit/`, with verification false;
explicit retained-contract required-success rejection is preserved separately at
`build/ci-policy-evidence/ae586e1-required-success/`.

Main now requires **80 strict app-bound checks**, adding only the four
`Foundation interrupt-*` cases. Readback preserves every unrelated protection
field in `build/ci-policy-evidence/interrupt-admission-policy-1791680279140833000/`.
The next published head needs its own complete 80-check execution; configuration
and policy updates do not establish remote success. No failing CPU lane is removed
or relaxed. Local QEMU 11.1.2 remains portability evidence against frozen 11.1.1.
Full roadmap scope, the original **51/100** console count, Experimental status and
draft/unmerged PR #9 remain unchanged. Wider ownership/IRQ review, 24-hour mixed
pressure, frozen-reference qualification and independent review remain open.

## October 13 host coverage feedback slice

The interrupted F6.2 harness work was completed locally on a dirty checkout
based on `ef2bc6812e53747733536c874a9f13ade66ae9ac`. The host-only
[`coverage-fuzz` lane](../tools/coverage-fuzz/README.md) verifies every source
file against the LLVM 21.1.8 commit and SHA-256 lock, builds its own libFuzzer
runtime, instruments the Rust target and production-linked parser code with
SanitizerCoverage, and retains per-target corpora, logs, summaries and crash
artifacts under `build/coverage-fuzz/`.

Ten targets each completed 5,000 cases without a crash: ELF, native ELF, IPv4,
IPv6, UDP, TCP, sockets, boot maps, paths and RX/TX descriptors. Final edge
coverage ranged from 12 to 210; the socket target reported 797 features.
The retained logs show coverage-selected corpus growth on multiple targets;
the TCP target added none after its earlier run had already found four. These are
libFuzzer counters for this host build, not whole-kernel coverage or a stability
claim. `cargo test --workspace --offline --locked`, the parser-stress CLI tests,
strict parser-stress Clippy, formatting and Markdown-link validation passed.
No QEMU, DMA or CPU qualification was run for this host-only harness change.
The remaining F6.2 targets and the wider roadmap gate remain open.

The reviewed lexical inventory changes from 405 sites/107 sources to 407
sites/110 sources. The only new unsafe function and block are the host libFuzzer
callback and its `from_raw_parts` conversion. The runtime supplies a readable
immutable byte range for this synchronous call; the callback caps length at
65,536, handles zero length without dereference, rejects a null nonempty pointer,
and does not retain the slice. No kernel unsafe operation, assembly or
Send/Sync implementation changes in this slice. The inventory records this
source context; it does not certify libFuzzer or its caller.
