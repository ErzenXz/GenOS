# GenOS roadmap

**Updated: 2026-09-22. Foundation series after `eb55347`; exact tested commits are in [VERIFICATION.md](docs/VERIFICATION.md). Current level: Experimental.**

GenOS is an independent Rust operating system. The immediate product goal is a
reliable kernel and useful terminal on a precisely defined reference machine.
Graphical UI comes after that console milestone. “Best” means measurable correctness,
recovery, latency, resource use and maintainability for supported workloads; it is
not a claim of universal superiority or absence of bugs.

This refresh reconciles the code, retained local tests, older roadmap entries and
[primary-source research](docs/research/README.md). The September 8 refresh changed
the plan; the September 22 foundation work adds implementation and tests.
Previous stage numbers remain for links and history. The
[previous roadmap](docs/history/2026-09-08-roadmap-before-refresh.md) is archived.

## Status language

- **Implemented, local evidence:** the named behavior has code and a retained local proof.
- **Integrated:** the exact change is merged and its required remote checks passed.
- **Partial:** a bounded slice exists; remaining criteria are listed explicitly.
- **Planned:** required implementation or evidence is missing.
- **Blocked:** a named dependency or external permission prevents progress on that item.
- **Deferred:** outside the current target; not a hidden release prerequisite.

`[x]` below records only the scoped implementation/evidence stated beside it.
It does not close its containing gate, establish remote CI enforcement, or raise
the release level. An untested or unpublished requirement stays `[ ]`. Test counts,
markers, architecture names and programming languages are not safety certifications.

## Target and release sequence

| Milestone | Required result | Scope |
| --- | --- | --- |
| **R1 — Verified kernel reference** | F0–F7 and applicable S1 failure-semantics gates pass, with explicit assumptions and supported limits | One admitted CPU; pinned x86_64 QEMU/firmware profile; real isolation, ownership, fault and cleanup evidence |
| **R2 — Useful console preview** | R1 plus S1/S2 and C1–C4 | Native applications, usable storage, streams, editing, process control and recovery through the terminal |
| **R3 — Stable console reference** | R2 plus C5 and Stage 10's console qualification | Repeatable local developer workflows and long-run reliability on the named VM; no GUI dependency |
| **Hardened/network profile** | R1 plus applicable Stage 6 gates; 5.4E/5.5 for general network claims | Explicit threat model, trusted distribution, reviewed crypto and supported network behavior |
| **Physical console reference** | R3 plus selected Stage 8 device/hardware gates | One named physical machine, with its own firmware/device/recovery evidence |
| **Graphical product** | R3 and the application/security/device contracts needed for that graphical profile | Stage 9; UI remains in userspace and cannot replace the recovery terminal |

R3 is a **product acceptance milestone**, not a replacement for the engineering
release levels in [ENGINEERING_QUALITY.md](docs/ENGINEERING_QUALITY.md). It may be
qualified as a local/offline developer console while broader networking and hardware
remain unsupported. Credentials, hostile workloads, automatic updates and production
services require their additional security gates. Optional hardware features do not
all have to exist before a narrowly scoped console release can be dependable.

The initial reference is x86_64 QEMU, 512 MiB RAM, one admitted kernel CPU,
UEFI boot and serial I/O. The [versioned VM candidate](docs/REFERENCE_VM.md)
pins machine/CPU settings, tool versions, firmware identity and device/network
layout, with explicit no-NIC/network variants. It is not a qualified release
profile; external network responses are not deterministic fixtures. More RAM,
additional CPUs and physical devices are separate tested profiles.

## Engineering choices informed by research

These are planning directions for GenOS; candidate mechanisms still require their
own implementation decision and tests.

| Direction | Why it fits this project | Research |
| --- | --- | --- |
| Keep the Rust monolithic kernel and deepen its modules | Preserve working code while reducing ownership/unsafe coupling; no evidence currently justifies a rewrite | [Kernel foundations](docs/research/2026-09-kernel-foundations.md) |
| Enforce frame grants and eagerly preserve a bounded CPU state set | Make ownership and process isolation auditable before optimizing context switches | [Kernel foundations](docs/research/2026-09-kernel-foundations.md) |
| Native capability-explicit spawn, namespaces and bounded streams | General applications and composability can build on existing handles without implementing all of POSIX | [Console platform](docs/research/2026-09-console-platform.md) |
| Specify failure outcomes before choosing filesystem growth | Durability, uncertain commits and recovery cost matter more than the name of the data structure | [Storage research](docs/research/2026-09-console-platform.md) |
| Layer host/property tests, selected Miri/model checks, fuzzing and real CPU/device tests | Each catches different failures; no single pass proves the whole OS safe | [Verification research](docs/research/2026-09-kernel-foundations.md) |
| Prefer a versioned VirtIO reference and a conservative TCP baseline | Finish the real device/protocol lifecycle before multiqueue/offloads or algorithm experiments | [Network/hardware research](docs/research/2026-09-network-hardware.md) |
| Reuse maintained reviewed crypto/trust components when their platform prerequisites exist | Avoid custom cryptography and separate update verification from durable activation | [Trust research](docs/research/2026-09-console-platform.md) |

## Current baseline: GenOS 0.56

| Area | What exists now | What the evidence does not establish |
| --- | --- | --- |
| Boot and exceptions | UEFI map/handoff admission; BSP/reentry guard; 2 MiB owned kernel stack; normalized exceptions; fourteen stack guard pages; protected IDT | Exhaustive firmware map/exit retries, stack high-water/nesting/return qualification or complete CPU-state qualification |
| Page protection | NX/WP and supported SMEP/SMAP; user mapping and linked kernel section permissions | Physical-frame-wide W^X across aliases or every CPU feature combination |
| Memory | Bitmap grants, rollback, zero-before-grant, scrub-before-release, scoped IRQ access, `mem` counters | Caller/owner tokens, shared/pinned-frame retirement or all shared-state synchronization |
| Processes and authority | Ring 3, preemption, private eager x87/MMX/SSE state, typed handles, exact deferred request identity, lifecycle cleanup | General spawn/heap/streams, tailored namespaces, full CPU-state qualification or stable application compatibility |
| Modules | Host-tested endpoint and pathname policy modules; lexical unsafe inventory | Complete decomposition, caller-invariant audit or automatic semantic safety proof |
| Terminal | Quiet normal session; bounded `help`; working serial `clear`; file/job commands and `mem` | Working directories, full line editing, quoting, pipelines, general launch, foreground cancellation or guest shutdown |
| Storage | Bounded GFS2 snapshots, ATA/PCI discovery, host inspection/repair, uncertain-commit write quarantine and atomic RAM mount | General filesystem capacity, live reconciliation, exhaustive device fault models or production data safety |
| Network | Modern VirtIO/MSI-X, IPv4, bounded concurrent TCP, socket waits and fault tests | General Internet TCP behavior, multi-process fairness at scale, arbitrary streams or secure traffic |
| IPv6 | SLAAC, DAD, neighbor/control parsing and router echo in the reference network | IPv6 application sockets, AAAA, IPv6-only boot or complete host conformance |
| SDK | A separately built native ELF executes and is reclaimed; ABI mismatch handling exists | General named launch, packages or a maintained ABI/SDK compatibility promise |
| Verification | Host tests, parser CLI/mutation tests, scoped QEMU suites, negative harness tests and retained per-run evidence | New remote CI runs, 1000-boot evidence, sustained qualification, whole-kernel verification or real-hardware support |

Evidence, test totals and exact commits: [VERIFICATION.md](docs/VERIFICATION.md).
The September 8 research refresh was documentation-only; September 22 records
implementation and its separate verification. Current source, limitations and evidence
must agree before an item can move to Integrated.

Current bounds matter: 4 managed asynchronous process slots including the shell;
20 unified handles per process including 4 file and 4 socket handles; 32 VFS nodes;
512-byte files; 64-byte paths; 80-byte console writes; 128-byte socket queues;
4 global passive slots with a 2-slot per-owner ceiling; up to 8 GiB of managed usable
frames across 64 ranges. These are current limits, not the final product design.

## Immediate priority: foundation correctness gate

**Status: partial. No foundation-wide completion or stable-release claim.**

| Next work item | Why it comes next | Completion evidence |
| --- | --- | --- |
| **F0.1 — Publish verification** | The VM candidate is pinned locally; GitHub rejected workflow publication for missing OAuth `workflow` scope | Authorized publication, exact-head remote checks and independent reproduction |
| **F1.2 — CPU state and stack containment** | General applications must not share register state or corrupt adjacent stacks | Adversarial XSTATE tests, guard faults and explicit nesting/return-fault policy |
| **F3.1/F2.1 — Enforce frame and alias ownership** | A live bitmap bit is not proof that the caller may free or remap it | Wrong-owner/stale/pinned/alias cases denied; hardware access tests; retirement before reuse |
| **S1.2/S1.3 — Expand storage failure qualification** | S1.1 now quarantines uncertain commits; the host device model is bounded | Broader sector loss/reordering, device errors and independent recovery evidence |
| **F4/F5 — Extract and guard remaining state** | Broader applications multiply the current raw-global assumptions | Executable ownership interfaces, context/lock assertions and delayed-interrupt tests |
| **F6 — Coverage and sustained failures** | Mutation counts and short boots leave important state spaces unexplored | Coverage-guided targets, repeatable fault schedules, retained regressions and sustained-run accounting |
| **C1 → C2/C3 → C4 → C5** | Build application/stream/storage contracts before shell syntax depends on them | Complete visible console workflows, then stability qualification |

Work can proceed in parallel where interfaces are independent. Fixes to current
correctness defects and required test seams are allowed before R1; broad features
must not deepen a known ownership or isolation violation.

### F0 — Green, reproducible verification

**Owner:** build/test tooling. **Status:** local checks implemented; integration blocked.

- [x] Pinned Rust 1.97.0 and declared 1.97 minimum; host/target builds and strict lint commands exist.
- [x] Separate normal debug/release, validation, no-NIC, network, storage and CPU proof commands exist locally.
- [x] Serial logs, image/source identities, fixture patches and scoped manifests exist; new CI jobs are configured in the local branch.
- [x] A versioned VM candidate and exact tool/firmware preflight exist; new normal evidence requires a fresh serial challenge, ordered readiness, unique retained run files and failure details. CPU-state and exception proofs reject duplicate/wrong-phase records.
- [x] September 22 local checks passed the complete suite, six malformed-boot cases, five BSP cases, eight original exceptions, strict reference acceptance and ten consecutive release boots. [Exact commits and limits](docs/VERIFICATION.md#september-22-foundation-implementation-and-verification) are part of the claim; no release level advanced.
- [ ] **F0.1:** resolve authorized workflow publication; merge only after required checks pass on the exact reviewed head. Do not remove checks to bypass the permission restriction.
- [ ] **F0.2:** pin/reference the compiler, QEMU machine/CPU, firmware hash, disk/device layout and test network; define the supported feature matrix and upgrade procedure.
  The candidate settings and upgrade procedure are implemented in [REFERENCE_VM.md](docs/REFERENCE_VM.md). Broader feature-matrix qualification and reproducible artifact acquisition remain open.
- [ ] **F0.3:** verify all required CI lanes actually run independently and are required by repository/release policy. A warning in one lane must not hide results from the others.
- [ ] **F0.4:** test the harness itself: omitted, duplicated, forged, wrong-phase and stale success evidence must fail. Retain incomplete/failing manifests and stderr, not just successful runs.
  Normal boot, CPU-state, boot-map, exception and BSP parsers have focused negative tests. Remaining legacy validation/network substring checks still need conversion.
- [ ] **F0.5:** reproduce clean builds on the supported host lanes, publish artifact hashes/provenance and make another reviewer reproduce the documented reference run.

### F1 — Complete exception and interrupt entry

**Owner:** architecture/context modules. **Status:** base entry delivered; extended context/containment open.

- [x] Normalized vector/error/frame handling, explicit user termination/kernel halt, dedicated emergency stacks and IDT write protection have scoped reference proofs.
- [x] Eight original user/kernel exception cases and six page-protection probes are reported as passing; BSP/reentry checks and the owned boot-stack fix are implemented.
- [x] Checked UEFI byte decoding and kernel handoff admission reject descriptor truncation/capacity, overflow, overlap and unretained boot/kernel/initrd memory. See [MEMORY.md](docs/MEMORY.md) for the trusted-pointer boundary and rejection fixtures.
- [x] The [bounded eager CPU-state policy](docs/CPU_STATE.md) preserves x87/MMX/XMM0–15/MXCSR, initializes fresh process state, disables OSXSAVE/PKE and enforces soft-float kernel compilation. A real Ring 3 fixture covers six processes, direct/yield syscalls, preemption, fault/reuse and exact frame reclamation.
- [ ] **F1.1:** validate complete BootInfo and firmware maps: descriptor count/stride/version, checked ranges, map growth, stale exit keys, overlap and reserved kernel/firmware/device memory. Capacity exhaustion must not silently truncate.
  Map/handoff checks are implemented; the pinned UEFI helper supplies one stale-key retry. A [bounded kernel ELF load plan](docs/KERNEL_ELF.md) now validates the complete image before allocation/copy. Exhaustive map-growth/exit fault injection and firmware allocation-failure qualification remain open.
- [ ] **F1.2:** inventory all process-visible register state. Implement a bounded, CPUID-validated eager XSTATE save/restore policy, with a justified narrower fallback; define initial state and kernel FPU/SIMD use. Test every enabled component through preemption, syscall, fault and reuse.
  The fixed 512-byte FXSAVE64 fallback is implemented and has scoped VM evidence. Broader CPU state (including FS/GS/debug contracts), unmasked FP exceptions and physical/feature-matrix qualification remain open; no complete F1.2 claim.
- [ ] **F1.3:** give boot, privilege, interrupt and emergency stacks inaccessible guards and usage measurements. Overflow must reach controlled containment instead of adjacent corruption or unexplained reset.
  All seven stacks now have [fourteen page guards](docs/KERNEL_STACKS.md), installed/read back before IRQ enable. Selected real fault fixtures are provided; high-water measurements, nesting and guard-skipping qualification remain open.
- [ ] **F1.4:** specify and test NMI/machine-check/same-IST nesting, fault-during-return and malformed return state. Recover only where a safe recovery contract is demonstrated; otherwise halt deliberately.
- [ ] **F1.5:** test missing/mixed CPU features and unsupported ISA use before admitting general applications. Record exactly what the reference CPU contract permits.

Research basis: [kernel foundations](docs/research/2026-09-kernel-foundations.md).
F1's earlier checked entries described the delivered entry slice; they did not close
these remaining CPU-state and hardware obligations.

### F2 — Hardware-enforced page protections

**Owner:** mapping and architecture modules. **Status:** protection bits and selected mappings proven; alias policy partial.

- [x] Required NX/WP and supported SMEP/SMAP are enabled/read back; linked sections, IDT, user data and stacks receive explicit permissions.
- [x] Current explicit mapping APIs reject W+X; real faults test the selected protections.
- [ ] **F2.1:** enforce a physical-frame permission/alias policy across the kernel direct map, temporary loader mappings, user aliases, remapping and protection changes. A writable alias must not silently defeat executable/read-only authority.
- [ ] **F2.2:** centralize user-copy lifetime/range validation and mapping updates behind reviewed interfaces; retire translations before backing storage can be reused.
- [ ] **F2.3:** extend actual CPU access tests across every mapping family and feature profile, including stale translations, cross-process aliases and failed protection changes.

### F3 — Transactional physical and virtual memory

**Owner:** allocator/address-space modules. **Status:** bounded owner/generation grants, mapping pins and rollback delivered; sharing/device policy incomplete.

- [x] Lossless bitmap reclamation, fragmented-map tests, zero-before-grant, scrub-before-release and scoped IRQ allocator access exist.
- [x] All ten reference process-construction allocation cutoffs restore the live-frame baseline; `mem` observes an increase for a running job and return after cleanup.
- [ ] **F3.1:** make grants carry enforceable owner/allocation identity; define explicit sharing and pinned device buffers. A physical address alone must not authorize release.
  [Private-field frame grants](docs/FRAME_GRANTS.md) now enforce owner/allocation identity and reject stale reuse. Explicit sharing and device pins remain open; user aliases are denied.
- [ ] **F3.2:** retire references, mappings, translations and device use before scrubbing/reuse. Wrong-owner, stale-generation, duplicate, aliased and pinned releases must leave state and bytes unchanged.
  User PTE pins, inactive-root construction/teardown, CR3 retirement with PCID/global retention disabled, and unlink-before-release are implemented. Raw kernel references/direct aliases, DMA and SMP still require broader lifetime policy.
- [ ] **F3.3:** define contiguous/ordered allocation and early-boot versus runtime allocation contracts; measure metadata overhead and maintain explicit RAM/region ceilings.
- [ ] **F3.4:** audit existing constructors/destructors and failure boundaries, including page-table splitting, partial ELF load and handles. Recoverable failure must leave no leaked authority or frame. Apply the same gate when C1 later introduces heap/mapping growth; R1 does not require that later application feature.
- [ ] **F3.5:** add bounded memory-pressure behavior, per-owner accounting and a coherent per-open or versioned-retry diagnostic snapshot; `/MEMORY.STATUS` must stay coherent across partial reads.
  Immutable per-open snapshots and kernel/user grant accounting now exist, with explicit 8192-live-grant capacity and failure reporting. Per-process quotas and pressure policy remain open.

Contract and current limits: [MEMORY.md](docs/MEMORY.md). Scrubbing is RAM hygiene,
not proof of cache erasure or physical remanence protection. Owner/grant enforcement
and its bounded scope are described in [FRAME_GRANTS.md](docs/FRAME_GRANTS.md).

### F4 — Kernel ownership and decomposition

**Owner:** each subsystem maintainer. **Status:** endpoint/path seams extracted; wider decomposition open.

- [x] Endpoint authority and canonical pathname policy execute in host-tested modules; a lexical unsafe/assembly inventory retains source context.
- [ ] **F4.1:** split process state, CPU context, scheduler policy, ELF loading, user-copy, lifecycle, syscalls and remaining typed handles into modules with clear failure/cleanup interfaces.
- [ ] **F4.2:** classify every mutable object as boot-only, coordinator-owned, IRQ-shared or emergency-safe; replace raw global mutation with scoped access that cannot leak mutable borrows.
- [ ] **F4.3:** keep rendering/terminal presentation outside storage, scheduling, lifecycle and transport ownership. Add dependency checks that fail on forbidden imports/mutation paths.
- [ ] **F4.4:** review caller obligations, aliasing, synchronization, assembly clobbers and failure containment at every unsafe boundary, including unsafe Send/Sync. Inventory presence alone does not close this review.
- [ ] **F4.5:** document ABI, scheduler, interrupt, storage and driver decisions, compatibility and rollback; remove or migrate dormant tests so they really compile against production modules.

### F5 — Explicit single-core and concurrency model

**Owner:** architecture, scheduler and device coordinators. **Status:** BSP admission and allocator critical sections delivered; global audit incomplete.

- [x] Non-BSP/repeated entry is rejected; one/four-vCPU reference tests still admit one kernel CPU.
- [x] IRQ rules and scoped allocator access exist; nested IF preservation has a real CPU probe.
- [ ] **F5.1:** guard all state shared with interrupts, and assert allowed execution context and lock order in debug/validation builds. No handler may wait on a lock held by the interrupted context.
- [ ] **F5.2:** bound IRQ work and masked duration; move blocking/expensive device work to coordinators. Record worst observed latency under the declared workload.
- [ ] **F5.3:** put current process/address space/scheduler-local/interrupt-local state behind a per-CPU-ready interface, while retaining exactly one active CPU in R1.
- [ ] **F5.4:** test delayed/nested interrupt delivery during allocation, context changes, request cancellation and device completion, with forward progress and ownership checks.
- [ ] **F5.5:** define local TLB retirement now and the future acknowledged cross-CPU shootdown contract. Actual SMP implementation belongs to Stage 8 and cannot be enabled early.

### F6 — Test boot, release boot, fuzzing, and fault injection

**Owner:** subsystem test owners and harness tooling. **Status:** useful local layers; coverage/long-run gates incomplete.

- [x] Explicit normal/validation policies; normal interactive tests use visible results and reject development trace leakage.
- [x] Retained deterministic ELF/network/socket mutation inputs, exact fault evidence parsers, allocation injection and selected packet/storage failures exist.
- [ ] **F6.1:** finish the normal startup cost/dependency audit. Optional devices must fail within declared deadlines and cannot make the local terminal depend on a test server or fixture.
- [ ] **F6.2:** add coverage-guided targets for boot maps, ELF, paths, partitions/snapshots, Ethernet/ARP/IP/ICMP/UDP/DHCP/DNS/TCP, descriptors and syscall/handle operation sequences. Preserve every fixed counterexample with an invariant and replay command.
- [ ] **F6.3:** run Miri on compatible pure Rust modules; pilot bounded model checking on small ownership/generation models. Pin tool versions and expose unsupported cases. Neither tool is a whole-kernel proof.
- [ ] **F6.4:** expand deterministic allocation, copy failure, cancellation, partial I/O, power-loss, packet corruption/loss/reordering/zero-window, IRQ and device-reset schedules. Test recovery failing again.
- [ ] **F6.5:** execute the configured 1000-boot lane and retain successes, first failure and source/profile identity. Five earlier local boots do not satisfy it.
- [ ] **F6.6:** qualify the existing kernel fixtures with at least 10000 lifecycle cycles and a proposed 24-hour single-boot memory/IPC/I/O pressure run. Record bounded caches, resource baselines, deadlines and coverage gaps. C5 later repeats and extends this with general applications and the complete console workflow; R1 does not depend on those later features.

### F7 — Reviewable delivery process

**Owner:** change author and reviewer. **Status:** process defined; enforcement/evidence must accompany each release.

- [ ] **F7.1:** produce buildable, reviewable commits; separate mechanical moves from behavior where possible and explain inseparable large changes.
- [ ] **F7.2:** map each public guarantee to success, negative, exhaustion, cancellation and cleanup evidence; review the actual failing cases and unsafe sites.
- [ ] **F7.3:** retain migration/downgrade/rollback plans for every public format/interface and a release-specific limitations snapshot.
- [ ] **F7.4:** require reproducible performance experiments and independent clean-build review. Do not optimize by removing required correctness work or relaxing a failing budget after the fact.

### Foundation gate exit

R1 requires every remaining F0–F7 criterion applicable to the frozen single-core
reference profile, plus an honest bounded-storage failure contract under S1. A
review must record excluded CPU/device/security assumptions explicitly. SMP-specific
execution, general Internet operation and other hardware are not silently claimed
by passing this one profile. R1 alone is not a complete terminal product.

## Delivered experimental vertical slices

The historical Stages 0–5.4D delivered UEFI/kernel entry, framebuffer experiments,
Ring 3, capabilities, runtime ownership, bounded storage and bounded network slices.
Later work added concurrent TCP/MSI-X, IPv6 control, SDK execution, CPU protections,
transactional memory and terminal cleanup. Preserve those achievements in
[MILESTONE_HISTORY.md](docs/MILESTONE_HISTORY.md); do not repeatedly schedule them
as if missing or describe their narrow acceptance as production readiness.

## Stage 4 continuation — Storage integrity and useful capacity

**Status: partial. S1 fixes current failure semantics; S2 depends on S1 and F3/F4.**

SQLite's crash-testing discipline and littlefs's bounded recovery design are useful
references, not an automatic filesystem selection. See [storage/platform research](docs/research/2026-09-console-platform.md).

- [x] Dual snapshot format, synchronous commits, host inspection/repair and read-only recovery have bounded local proofs.
- [x] **S1.1:** reproduce unknown final-commit outcomes and enter read-only recovery. Explicit outcomes, sticky write quarantine, dirty-cache discard, last-acknowledged RAM rollback and visible remount status are implemented. The host regression proves either old or new complete media state after failed final flush; subsequent writes issue no device commands. Live reconciliation is not implemented.
- [ ] **S1.2:** specify atomic visibility, acknowledged durability, cancellation and remount results separately. Inject cuts/errors before and after every logical write/flush, including the final commit record.
  The production commit/cache path now runs 176 before/after device-operation error cases across writeback and writethrough models. See [STORAGE.md](docs/STORAGE.md) for exact guarantees and model limits; wider hardware/cancellation coverage remains.
- [ ] **S1.3:** test torn/reordered/lost sectors, corruption of either/both generations, full storage, counter overflow and a second failure during repair/recovery. Preserve unreadable media; never silently format it.
  Twenty-five torn/recovery cases, counter-overflow refusal and atomic mount rejection for late malformed entries or a full VFS are covered. Arbitrary lost/reordered sectors and physical-device qualification remain open.
- [ ] **S1.4:** document device flush/FUA/cache assumptions and compare guest recovery with an independent host checker. Killing QEMU alone is not a full physical power-loss model.
- [ ] **S2.1:** choose format growth in an ADR after comparing workloads, space amplification, RAM cost, recovery time, implementation/reuse/licensing cost and tooling. Do not select journaling, copy-on-write trees or littlefs by fashion.
- [ ] **S2.2:** support larger files/directories and streams with explicit quotas and partial-I/O semantics. Initial candidate tests: at least 1 MiB files, 1024 namespace entries and 255-byte complete paths; validate budgets before freezing C5.
- [ ] **S2.3:** implement atomic replacement/rename, metadata and filename policy, backup/export/restore, versioned migration and interrupted-migration recovery. Keep a read-only path for old GFS2 data.

## Stage 5.4E — Concurrent and production-oriented TCP

**Status: partial, not unstarted. Extension depends on R1 and C1 stream/authority contracts.**

- [x] Modern VirtIO/MSI-X, bounded concurrent passive streams, per-owner limits, readiness waits and selected loss/reordering/congestion tests exist on the VM.
- [ ] **N1.1:** maintain a requirement-to-test matrix for the declared TCP profile using RFC 9293 and companion timing/congestion specifications; identify valid unsupported features separately from malformed input.
- [ ] **N1.2:** finish general byte streams, multi-segment flight/reassembly, sequence wrap, duplicate handling, simultaneous/half-close, reset, persist/zero-window, RTT/RTO, congestion and cancellation behavior.
- [ ] **N1.3:** prove fairness across different process owners, not only clients of one listener; stalled/malicious peers must stay inside per-owner memory, work, retry and lifetime budgets.
- [ ] **N1.4:** use scripted wire/application/timing cases, packet capture and retained failure seeds. Include loss, delay, corruption, duplicate ACK/data, slow readers, peer death and exhaustion.
- [ ] **N1.5:** establish throughput/latency/CPU/memory/queue baselines before multiqueue, offloads, larger windows or alternative congestion algorithms become defaults.

Network methods and specification revisions: [network/hardware research](docs/research/2026-09-network-hardware.md).

## Stage 5.5 — IPv6 dual stack

**Status: control-plane foundation delivered; application/host completeness planned.**

- [x] Advertised-prefix configuration, DAD, selected neighbor/control validation and router echo have reference evidence.
- [ ] **N2.1:** add IPv6 socket addressing/demultiplexing, UDP/TCP applications, DNS AAAA, address selection and IPv6-only initialization.
- [ ] **N2.2:** complete supported neighbor/reachability, router/route lifetime, extension/fragment policy, ICMPv6 error and path-MTU behavior against the declared node profile.
- [ ] **N2.3:** test IPv4-only, IPv6-only and dual-stack networks, malformed input, expiry/conflict/recovery and preservation of existing IPv4 behavior.
- [ ] **N2.4:** publish privacy/address-identifier policy; extend DHCPv6/RDNSS/multi-router behavior where the selected supported network requires it.

A router echo does not close these gates. `net` is a configuration diagnostic,
not evidence of Internet or TLS connectivity.

## Stage 6 — Security, identity, and trusted distribution

**Status: planned beyond current capability/protection foundations. Required before corresponding hardened, sensitive-data or network claims.**

- [ ] **SEC1:** define attacker/firmware/device/CPU assumptions; audit cross-process, filesystem, service and network authority. Native spawn should grant only explicit capabilities.
- [ ] **SEC2:** provide cryptographic entropy/reseed policy, local user/service/session identity where supported, permissions, attenuated delegation, sandbox profiles and resource limits.
- [ ] **SEC3:** integrate maintained reviewed TLS 1.3/crypto in userspace after streams, entropy, trust and time exist. Test hostname/path/signature/expiry failures; no plaintext downgrade for credentials, updates or personal data.
- [ ] **SEC4:** define trust-root provisioning, versions/expiry, key rotation/revocation, target compatibility and persistent anti-rollback state. Evaluate a maintained TUF implementation; signatures alone do not establish an update system.
- [ ] **SEC5:** separate download, verification, installation, activation and boot-health confirmation; test interruption at each step and retain an authorized recovery path. A/B images are a candidate, not a mandated layout.
- [ ] **SEC6:** publish unsafe/threat review, dependency/provenance inventory, supported versions, response policy and mitigation limits. Secure/measured boot, secrets storage and IOMMU claims require their own implementation and hardware evidence.

An explicitly offline, developer-built console milestone need not ship TLS, multiuser
login or automatic update clients. That exclusion must remain visible in its profile;
it cannot be used to market untrusted-network or sensitive-data readiness.

## Stage 7 — Stable application and service platform

**Status: SDK demonstration delivered; console platform planned. R2/R3 are the immediate product path.**

### C1 — Native application, memory and stream contracts

- [x] An SDK example builds outside the repository, checks ABI compatibility, executes in Ring 3 and is reclaimed.
- [ ] Define native capability-explicit spawn with executable reference, arguments, optional environment, working-directory authority, stdin/stdout/stderr and resource budget. Validate the whole request before making the child runnable.
- [ ] Add owned userspace heap/mapping growth and bounded accounting; publish an ABI/image compatibility policy and deterministic unsupported-version errors.
- [ ] Add bounded streams with partial I/O, readiness, EOF, broken-peer errors, cancellation and close. Keep structured message/handle transfer separate where useful.
- [ ] Launch at least three independently built useful programs from storage. Test invalid ELF/ABI, every construction failure, stale/wrong-rights handles and cleanup of all intermediate resources.
- [ ] Define supervised process groups, service discovery, restart budgets and recovery-console availability; a failed service or restart storm must not destroy unrelated authority.

Static native ELF programs are the starting point. POSIX conformance, `fork`, a C
library, shared libraries and dynamic linking are optional later compatibility
choices, not prerequisites for GenOS's own launch/stream model.

### C2 — Terminal input and command execution

- [x] Normal output is quiet; `help` respects the 80-byte ABI limit; `clear`, files/jobs and `mem` have visible transcript tests.
- [ ] Separate byte/escape decoding, bounded line editing, shell parsing and foreground input ownership. Cover cursor/delete/home/end, history, completion, cancellation, paste, long input and invalid/incomplete sequences.
- [ ] Add `cd`, `pwd`, relative paths, quoting/escaping, general named launch and exit-status reporting. Enforce directory confinement below the shell; text normalization is not authority.
- [ ] Define encoding, filename comparison and safe display of control bytes. Data display must not inject terminal controls, alter pending input or execute commands; prompt ownership must remain clear after background output. A limited initial encoding profile must be documented.
- [ ] Build copy, atomic move/rename, text viewing/editing and diagnostics around real storage/stream interfaces. Add `ping`/DNS tools when a reviewed network interface exists; the ABI's existing test `ping` is not ICMP.

### C3 — Composable jobs and scripts

- [ ] Implement redirection and multi-stage pipelines after C1 streams; define pipeline exit status and partial-failure cleanup.
- [ ] Prove transfers larger than every pipe buffer with slow/early-exiting consumers, peer death and cancellation while blocked; the prompt must return without deadlock.
- [ ] Add foreground/background ownership, whole-pipeline interruption and bounded job records; restore the terminal when a process exits or dies after changing its mode.
- [ ] Add a small documented scripting language, startup/configuration handling and actionable errors. Do not advertise shell syntax without the corresponding execution semantics.

### C4 — Session, shutdown and recovery

- [ ] Implement guest `exit`, shell/session restart, shutdown and reboot with privilege checks, bounded service stopping, storage draining and explicit failure outcomes.
- [ ] Provide a functional serial recovery parser for inspection, read-only mount/export, backup/restore and authorized repair. Do not depend on graphics or the normal shell surviving.
- [ ] Test a failed shell, failed optional service, damaged configuration, unavailable device, failed flush and interrupted recovery; return to a usable documented state.
- [ ] Keep QEMU host controls separate from guest shutdown. Exiting the emulator is not a guest durability guarantee.

### C5 — Stable console reference acceptance

**Status: proposed targets, all unexecuted as a complete qualification.** Freeze the
profile and numeric budgets before the candidate run; changing a target requires
rationale and retained earlier failures. These are GenOS engineering targets, not
thresholds supplied by a standard or proof that no defects remain.

- [ ] R1, S1/S2 and C1–C4 pass; publish the exact supported storage/network/encoding/security profile and all excluded capabilities.
- [ ] Demonstrate an end-to-end workflow: boot, edit/cancel commands, create a project, edit/save/reopen files, launch native programs, pipe data, cancel a stuck workload, recover the shell, reboot and verify durable bytes.
- [ ] Exercise at least eight application processes plus the shell within the 512 MiB reference workload; publish actual configurable quotas. This is a workload floor, not an architectural ceiling.
- [ ] Complete 1000 fresh boots; at least 10000 launch/exit/fault/kill/reap cycles in sustained runs; and a 72-hour mixed terminal/storage/process workload on one boot.
- [ ] Require zero unexplained reset, hang, mixed committed state, authority leak or post-warmup unbounded resource growth. Use exact resource baselines and bounded-cache explanations, not aggregate “looks stable” logs.
- [ ] Run the storage failure/recovery matrix independently of normal workloads, plus the applicable network/device matrix. Successful writes survive remount under the stated device fault model.
- [ ] Measure cold/warm boot-to-input-ready, idle CPU/wakeups/memory, input/command latency, syscall/process/stream/storage cost and worst observed IRQ-masked time. Publish raw samples, spread and correctness results; freeze budgets from evidence rather than inventing performance wins.
- [ ] Have an independent clean environment reproduce the candidate image/workflow and complete Stage 10's independent console release checklist. Use a dedicated test environment capable of the required uninterrupted soak; the current 180-minute scheduled job cannot establish a 24/72-hour run.

## Stage 8 — Modern hardware, SMP, and power management

**Status: planned expansion; no physical hardware or SMP claim today.** These are
separate profiles, not a demand to implement every device before the first VM console.

- [ ] **H-VM:** define modern VirtIO block/network/console profiles and device interfaces. Pin a reviewed specification revision and negotiated features; legacy ATA/PIC paths remain explicitly labeled until replaced and proven.
- [ ] **H-DMA:** enforce DMA buffer lifetime, access direction, visibility and reset quiescence. Unknown reset completion cannot authorize reuse. Test malformed descriptors, used indices, coalesced/lost interrupts and repeated reset.
- [ ] **H-PC:** choose one physical x86_64 machine after a discovery report; implement its ACPI/APIC/MSI-X, storage and input path, with NVMe/xHCI as target interfaces. Record firmware and device revisions, timeout/flush/reset and recovery outcomes.
- [ ] **H-SMP:** only after F5, implement AP startup/parking, per-CPU tables/stacks/state, run queues, synchronization and acknowledged TLB shootdowns. Test simultaneous mapping/lifecycle changes and delayed acknowledgements before page reuse.
- [ ] **H-IOMMU:** implement and verify the selected DMA-isolation policy before claiming containment against untrusted devices; document trusted-device assumptions until then.
- [ ] **H-POWER:** add supported idle states, shutdown/reset, then suspend/resume, thermal/battery and hotplug/error behavior as separate hardware workloads. Preserve storage and ownership through failures.

[Research](docs/research/2026-09-network-hardware.md) recommends VirtIO for the VM,
but does not certify a driver by name. The inspected VirtIO 1.2 document is CS01;
the existing 1.3 link identifies CSD01, a draft. Feature-specific revision/status
must be recorded. UEFI 2.11 and ACPI 6.6 are research baselines, not new support claims.

## Stage 9 — Userspace graphics and product experience

**Status: deferred until R3 and the required application/security/device contracts.**

- [ ] Userspace compositor/window server and isolated surfaces/capability transfers.
- [ ] Selected virtio-gpu/physical GPU profile with lifecycle and resource limits.
- [ ] Fonts/shaping/scaling, focus/input methods, keyboard operation, accessibility,
  clipboard and drag/drop with explicit authority.
- [ ] Userspace terminal, files, tasks, settings and launcher; serial administration
  and recovery remain available when graphics fails.
- [ ] Visual, interaction, isolation, memory and latency acceptance for the supported profile.

No product UI moves back into Ring 0. A graphical VM does not require every future
laptop or multicore feature, but it must meet the contracts of the devices it uses.

## Stage 10 — Production candidate and daily-use qualification

**Status: planned. Evaluated per supported product/profile, including console-only.**

The console release checklist below consumes implementation and test evidence; it
does not require an R3 label as an input. Passing C5's workload gates and this
checklist permits the R3 designation. Production promotion is a later, separate
review. This avoids making C5, R3 and Stage 10 prerequisites of one another.

### Independent console release checklist

- [ ] Review the R1, S1/S2 and C1–C4 evidence and C5 workload results against the exact advertised profile; verify all included guarantees and excluded capabilities.
- [ ] Reproduce upgrades, authorized rollback, backup/restore and recovery from prior supported versions, including interrupted activation and failure during recovery. For the first version, retain its tested initial-install/recovery path and freeze the future migration contract.
- [ ] Publish profile limits, ABI/storage compatibility, image hashes/provenance, dependency/trust assumptions, maintenance/support policy and unresolved risks.
- [ ] Have an independent environment reproduce the candidate image and console workflow; retain fault/coverage/resource evidence. Scheduled failures must block promotion or become explicitly tracked work, never disappear from the record.
- [ ] Compare performance only with equivalent named workloads/configurations and reproducible data. No universal-superiority or “perfectly safe” release language.

### Production promotion

- [ ] After R3, satisfy the Hardened preview requirements and the Stage 6/network/hardware gates for every advertised threat model and configuration, then pass the production-candidate review in the quality plan.
- [ ] For physical daily use, attach the selected Stage 8 machine/device/power and recovery results; for a graphical product, attach Stage 9's interface, isolation and accessibility results.

Stage 9 is required only for a graphical product. It is not a hidden prerequisite
for a qualified console-only release.

## Cross-cutting scorecard

Each candidate publishes a claim-to-evidence ledger: source/build/profile identity,
positive and negative tests, failure/recovery outcomes, counters before/after,
coverage limitations and independent reproduction. Missing evidence is marked
missing. Track correctness, authority, recovery, bounded work, performance,
maintainability, hardware behavior and user-visible workflows separately.

## Benchmarking against Linux or another system

Pin both versions and configurations; declare feature differences, hardware/device
models, workload source, warmup/cache policy, sample count, raw measurements,
spread and failures. Compare boot-to-usable-input rather than an earlier internal
marker. Measure the work a user requested. A faster result applies only to that
experiment and must not be obtained by omitting required isolation or durability.

## How roadmap changes are made

Every update names the code baseline, new evidence, affected task IDs, dependencies,
compatibility/rollback consequences and source revisions. Reconcile the limitations,
README, subsystem contracts and quality plan together; preserve historical decisions
as history. A checkbox cannot advance on a code change without its scoped evidence.
Research proposals do not count as implementation. No target is lowered merely to
turn a failing test green, and no release label advances because this document grew.
