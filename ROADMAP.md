# GenOS roadmap

GenOS is a from-scratch Rust operating system. The project is intentionally ambitious, but ambition does not replace evidence. This roadmap defines the order in which GenOS must earn correctness, security, reliability, performance, hardware support, and product quality.

The roadmap is a set of acceptance gates, not a feature wishlist. A stage is complete only when its observable criteria pass in automation or on documented reference hardware. Previous milestone labels describe delivered experimental slices. They do not imply production readiness.

## Status language

- **Delivered:** the scoped behavior exists and has a repeatable proof.
- **In progress:** implementation or required evidence is incomplete.
- **Planned:** the contract is defined, but implementation has not started.
- **Blocked:** work must not become the default path until its dependency gates pass.
- **Deferred:** intentionally outside the current product path.

Dates are omitted until contributor velocity makes forecasts useful.

## Engineering rules

1. Keep `main` green and bootable.
2. Fix correctness and security gaps before expanding product surface.
3. Build one reviewable vertical slice at a time.
4. Preserve explicit ownership across kernel, runtime, drivers, and userspace.
5. Treat firmware, devices, files, packets, and userspace pointers as untrusted input.
6. Make allocation, mutation, and cleanup transactional.
7. Bound interrupt work and all untrusted-input parsing.
8. Measure performance before making performance claims.
9. Keep legacy hardware behind explicit fallback policy.
10. Prefer a small proven contract over several partial contracts.

## Product goal and comparison policy

GenOS aims to become a small, coherent, inspectable operating system that can outperform larger systems on carefully defined workloads without sacrificing correctness.

“Better than Linux in every way” is not a valid engineering claim. Linux supports hardware, workloads, security policies, and compatibility requirements that GenOS does not yet attempt. GenOS may claim an advantage only for a named metric, workload, configuration, and baseline when the repository contains a reproducible harness and the result includes variance and failure cases.

Examples of valid future claims:

- lower boot-to-shell time on the same virtual machine configuration;
- lower idle memory or CPU use for the same reference service;
- smaller trusted or unsafe code surface for a defined feature set;
- lower process-launch latency under a published benchmark;
- simpler recovery behavior under a documented storage fault model.

See [the engineering quality plan](docs/ENGINEERING_QUALITY.md) for the evidence format and release levels.

## Current baseline: GenOS 0.49

The current experimental baseline includes:

- a repo-owned `x86_64` UEFI bootloader and versioned boot contract;
- a Rust `no_std` monolithic kernel;
- GDT, TSS, IDT, PIT/PIC interrupt setup, and serial diagnostics;
- separate Ring 3 address spaces, timer preemption, ELF loading, and ABI 17 syscalls;
- process-local typed capabilities for files, directories, endpoints, console access, process lifecycle, and sockets;
- an isolated Ring 3 serial shell and fail-closed emergency kernel console;
- RAM-backed temporary storage plus bounded persistent `/USER/` snapshots, inspection, repair, and read-only recovery;
- modern VirtIO 1.x networking with Ethernet, ARP, IPv4, ICMP, UDP, DHCP, DNS, bounded TCP clients, listener authority, one passive handshake, and one bounded accepted request/response transaction;
- host tests and QEMU smoke proofs for the implemented vertical slices.

These are real operating-system mechanisms. They remain constrained by the limitations tracked in [KNOWN_LIMITATIONS.md](docs/KNOWN_LIMITATIONS.md).

## Immediate priority: foundation correctness gate

**Status: in progress**

This gate blocks production or hardened-release language. It also blocks broad new product features that would deepen unsafe assumptions. Networking correctness work already required to close Stage 5.4 may continue only when it also advances this gate.

### F0 — Green, reproducible verification

Goal: every proposed change reaches all tests instead of failing early or depending on an undocumented local environment.

- [ ] `main` passes formatting, Clippy for every shipped target, workspace tests, image build, and QEMU boot.
- [ ] The supported Rust toolchain and minimum supported Rust version are explicit and tested.
- [ ] CI runs debug and release image builds where their behavior differs.
- [ ] Failure artifacts include serial output and enough configuration to reproduce the run.
- [ ] Required checks cannot be skipped by an earlier non-behavioral warning.
- [ ] The normal, no-network, deterministic-network, storage-recovery, and read-only-recovery boots have distinct required markers.

### F1 — Complete exception and interrupt entry

Goal: every architectural entry path constructs a valid frame, preserves required state, and terminates or halts deliberately.

- [ ] Replace the catch-all bare `iretq` entry with explicit stubs for exceptions with and without CPU-pushed error codes.
- [ ] Install handlers for all architecturally relevant x86 exceptions, including divide error, invalid opcode, debug, invalid TSS, segment-not-present, stack fault, alignment check, machine check, and control-protection fault when supported.
- [ ] Normalize vector, error code, instruction pointer, privilege level, stack, and fault address before entering Rust.
- [ ] Terminate the exact Ring 3 process for recoverable user exceptions without damaging another process.
- [ ] Print a complete serial fault record and halt on an unhandled Ring 0 exception.
- [ ] Handle spurious and unexpected external interrupts without returning through a malformed frame.
- [ ] Use dedicated interrupt stacks where architectural failure handling requires them.
- [ ] Make the initialized IDT read-only before enabling untrusted execution.

Acceptance proof:

- [ ] Deterministic Ring 3 tests exercise `#DE`, `#UD`, `#GP`, and `#PF` and leave a healthy peer running.
- [ ] Deterministic Ring 0 fault tests produce the expected serial frame and halt rather than looping or triple-faulting.
- [ ] No installed default entry consists only of `iretq`.

### F2 — Hardware-enforced page protections

Goal: make the page permissions promised by the loader true on the CPU, independent of firmware defaults.

- [ ] Discover NX, SMEP, SMAP, and related features through CPUID.
- [ ] Enable and verify `EFER.NXE` before mapping non-executable pages.
- [ ] Enable and verify `CR0.WP` so supervisor writes respect read-only mappings.
- [ ] Enable SMEP and SMAP when supported, with explicit guarded user-copy primitives.
- [ ] Reject writable-and-executable ELF mappings and preserve that invariant across every mapping API.
- [ ] Keep user stacks and writable data non-executable.
- [ ] Keep kernel text read-only and executable, kernel read-only data non-writable, and mutable kernel data non-executable once the linker and boot mappings expose those sections.

Acceptance proof:

- [ ] Executing from Ring 3 data or stack pages terminates only the offending process.
- [ ] Writing through a kernel read-only mapping faults under `CR0.WP`.
- [ ] A user mapping cannot execute in supervisor mode under SMEP, and ordinary kernel access cannot bypass SMAP unintentionally.
- [ ] Boot logs record the detected and enabled protection set without treating an unsupported optional feature as success.

### F3 — Transactional physical and virtual memory

Goal: every failed allocation leaves the exact pre-operation ownership state.

- [ ] Replace the fixed 256-frame recycle stack with a page-state allocator that can represent every managed frame.
- [ ] Track frame ownership and reject double free, foreign free, reserved-memory allocation, and aliasing.
- [ ] Support contiguous or ordered allocations only through an explicit contract.
- [ ] Roll back partial page-table cloning, user image loading, stack construction, and mapping failures.
- [ ] Define zeroing policy for newly granted user pages and reclaimed sensitive pages.
- [ ] Separate early-boot allocation from the normal allocator when their invariants differ.
- [ ] Publish allocator counters and consistency checks that remain usable without graphics.

Acceptance proof:

- [ ] Fault injection fails each allocation point in process construction and returns to the exact baseline frame count.
- [ ] Randomized host tests allocate and free across fragmented memory maps without duplicate ownership.
- [ ] Reclaiming more than 256 frames remains lossless.
- [ ] A failed address-space clone leaks no page-table frame.

### F4 — Kernel ownership and decomposition

Goal: make subsystem boundaries reviewable before concurrency and feature breadth multiply the state space.

- [ ] Split the current userspace implementation into process, context, scheduler, loader, user-copy, lifecycle, syscall, and typed-handle modules.
- [ ] Give each mutable state object one documented owner.
- [ ] Remove presentation code from scheduling, filesystem, lifecycle, and network completion paths.
- [ ] Inventory every `unsafe` block with its caller obligations and protected invariant.
- [ ] Replace cross-subsystem mutation through raw globals with narrow interfaces.
- [ ] Add architecture decision records for public ABI, scheduler, allocator, interrupt, storage-format, and driver-boundary decisions.

Acceptance proof:

- [ ] A source-boundary test prevents presentation code from mutating runtime-owned state.
- [ ] Module documentation identifies ownership, synchronization, failure behavior, and cleanup.
- [ ] A contributor can change one typed handle family without editing unrelated process-context or ELF-loader code.
- [ ] The unsafe inventory is generated or checked in CI and cannot silently shrink its review context.

### F5 — Explicit single-core and concurrency model

Goal: make the current single-core design safe now and prepare a deliberate path to SMP.

- [ ] Detect and reject accidental application-processor startup until SMP support exists.
- [ ] Document interrupt masking, nesting, preemption, and shared-state rules.
- [ ] Replace unsynchronized mutable globals with explicit single-core critical sections or IRQ-safe synchronization.
- [ ] Move current process, active address space, scheduler-local state, and interrupt-local state behind a per-CPU abstraction before starting a second CPU.
- [ ] Define lock ordering and which locks may be acquired in interrupt context.
- [ ] Add TLB invalidation and shootdown contracts before sharing address spaces across CPUs.

Acceptance proof:

- [ ] Static or host-side checks find no unguarded mutable global reachable from both normal and interrupt context.
- [ ] Nested or delayed interrupt tests preserve process and scheduler state.
- [ ] SMP remains disabled with an explicit diagnostic until per-CPU state and shootdowns pass their tests.

### F6 — Test boot, release boot, fuzzing, and fault injection

Goal: preserve deep validation without making a normal boot run development stress suites.

- [ ] Separate deterministic validation boot from the normal release boot through an explicit build or boot policy.
- [ ] Keep only cheap invariant checks in the release path.
- [ ] Move process-generation stress, rollback probes, parser corpora, and protocol fault suites into dedicated test modes.
- [ ] Add fuzz targets for ELF, boot contracts, filesystem snapshots, partition metadata, network frames, DNS, and TCP classifiers.
- [ ] Add deterministic allocation, I/O, packet loss, duplication, delay, reordering, reset, and cancellation injection.
- [ ] Run long boot and lifecycle repetition outside the fast pull-request lane and publish failures as artifacts.

Acceptance proof:

- [ ] A release boot reaches the shell without executing stress probes.
- [ ] A test boot proves the same subsystem contracts and fails when a required probe is removed.
- [ ] Fuzz targets retain regression inputs for every fixed crash or invariant violation.
- [ ] At least 1,000 repeated reference-VM boots and process lifecycles complete without leaked authority or memory before the hardened-preview label.

### F7 — Reviewable delivery process

Goal: make regressions discoverable and architecture decisions reversible.

- [ ] One commit contains one logical behavior or mechanical change.
- [ ] Every commit in a mergeable series builds and preserves the documented boot contract.
- [ ] Changes larger than roughly 500 non-generated lines explain why they cannot be split safely.
- [ ] Public contracts include migration and rollback plans.
- [ ] Performance changes include the exact command, environment, baseline, result, variance, and raw artifact.
- [ ] Security-sensitive changes identify the threat, authority boundary, negative tests, and unsafe code touched.

### Foundation gate exit

The foundation gate closes only when F0 through F7 pass on the reference VM and all remaining exceptions are documented as release-specific limitations. Closing it does not make GenOS production-ready. It permits the project to call the reference build **verified** and resume broader feature work on a safer base.

## Delivered experimental vertical slices

The following stages have delivered their scoped demonstrations. Their detailed contracts remain in the subsystem documents and commit history.

| Stage | Delivered scope | Status under this roadmap |
| --- | --- | --- |
| 0 | UEFI boot, kernel entry, boot contract, serial diagnostics, initial memory and interrupt setup | delivered experiment; F1-F3 remain release-blocking |
| 1 | framebuffer desktop, input, windows, terminal, RAM filesystem, task UI | delivered experiment; graphical product path deferred |
| 2 | Ring 3, address spaces, preemption, ELF loading, ABI, lifecycle, VFS, input, IPC, shell | delivered experiment; F1-F5 remain release-blocking |
| 3 | runtime ownership, unified typed handles, request identity, cleanup, headless serial path | delivered experiment; F4-F6 remain release-blocking |
| 4 | PCI-discovered ATA, partitioned bounded snapshots, cache, inspection, repair, read-only recovery | delivered bounded storage experiment |
| 5-5.3 | VirtIO 1.x, IPv4 stack, diagnostics, socket capabilities, asynchronous UDP | delivered bounded network experiment |
| 5.4A-D | bounded TCP client, listener authority, one passive handshake, one accepted transaction and close | delivered bounded TCP experiment |

Subsystem references:

- [userspace boundary](docs/USERSPACE.md)
- [runtime ownership](docs/RUNTIME.md)
- [storage format and recovery](docs/STORAGE.md)
- [networking contracts](docs/NETWORKING.md)

## Stage 5.4E — Concurrent and production-oriented TCP

**Status: blocked by the foundation gate; protocol design may continue**

Planned work:

- multiple simultaneous handshakes and accepted clients;
- fair listener and cross-process socket service;
- readiness waits and cancellation-aware scheduler wakeups;
- segmented long-lived streams, out-of-order reassembly, duplicate handling, half-close, and reset behavior;
- RTT estimation, retransmission timers, dynamic windows, congestion control, and bounded resource accounting;
- interrupt-driven VirtIO RX/TX through MSI-X, with polling only as bounded recovery;
- deterministic loss, delay, duplication, reordering, zero-window, slow-reader, reset, and exhaustion tests;
- reproducible throughput, latency, CPU-cost, recovery, and queue-occupancy budgets.

Acceptance criteria:

- [ ] Independent clients cannot share authority, bytes, readiness, or completion identity.
- [ ] One slow or malicious peer cannot starve another process or exhaust unbounded kernel memory.
- [ ] Transfers remain correct under the deterministic network fault matrix.
- [ ] Normal RX and TX completion is interrupt-driven.
- [ ] Published benchmarks identify payload size, concurrency, loss model, CPU cost, and memory use.

## Stage 5.5 — IPv6 dual stack

**Status: planned; required before production-network language**

Planned work:

- IPv6 validation, routing, extension-header policy, and path-MTU behavior;
- ICMPv6, neighbor discovery, duplicate-address detection, router advertisements, and SLAAC;
- DNS AAAA handling and address-selection policy;
- IPv4/IPv6 socket semantics without hiding address-family differences;
- deterministic IPv4-only, IPv6-only, and dual-stack reference networks.

Acceptance criteria:

- [ ] GenOS configures a usable IPv6 address and route without a hard-coded guest address.
- [ ] DNS, UDP, and TCP complete on IPv6-only and dual-stack networks.
- [ ] Malformed extension headers, advertisements, fragments, and ICMPv6 messages fail closed.
- [ ] IPv4 behavior remains covered by the same regression matrix.

## Stage 6 — Security, identity, and trusted distribution

**Status: planned; depends on the foundation gate**

Planned work:

- cryptographic entropy and a documented CSPRNG reseed policy;
- user, service, and session identities;
- filesystem permissions and capability delegation with attenuation;
- process sandbox profiles and explicit device/network authority;
- a versioned trust store and secure time policy;
- userspace TLS 1.3 using reviewed cryptography and test vectors;
- certificate path and hostname verification with negative interoperability tests;
- signed packages, update metadata, rollback protection, and transactional install;
- secure-boot research, measured-boot hooks, and secrets storage;
- threat models for every trusted boundary.

Acceptance criteria:

- [ ] Applications receive only explicitly granted resources.
- [ ] A compromised unprivileged process cannot read or modify another process, kernel memory, storage outside its authority, or unrelated network endpoints.
- [ ] Credentials, packages, updates, and personal data cannot silently downgrade to plaintext.
- [ ] Package and update signatures are verified before mutation.
- [ ] The release publishes threat models and an unsafe-code review report.

## Stage 7 — Stable application and service platform

**Status: planned**

Planned work:

- a versioned application ABI and compatibility policy;
- file-descriptor or stream abstractions where they improve composability without weakening capabilities;
- service discovery and capability delegation without raw PID authority;
- shared-memory and event primitives with explicit ownership;
- application manifests, packages, transactions, and SDK tooling;
- resource accounting, quotas, background-execution policy, and service supervision;
- an external application build that does not depend on private repository internals.

Acceptance criteria:

- [ ] An application builds outside this repository with the published SDK.
- [ ] ABI incompatibility produces a deterministic error or supported migration path.
- [ ] Installation and removal are transactional and verifiable.
- [ ] Service failure does not corrupt another service's state or authority.

## Stage 8 — Modern hardware, SMP, and power management

**Status: planned; depends on F5**

Required baseline:

- ACPI discovery and power control;
- local APIC/x2APIC, I/O APIC, MSI, and MSI-X;
- SMP startup, per-CPU state, scheduler scaling, and TLB shootdowns;
- modern VirtIO block, network, console, and GPU devices in the reference VM;
- NVMe for the first physical-storage reference;
- xHCI and USB HID for the first physical-input reference;
- IOMMU and DMA-isolation policy before untrusted-device support;
- PCIe capability, reset, power-state, hotplug, and error policy;
- suspend, resume, battery, thermal, and idle-state support.

Acceptance criteria:

- [ ] The reference VM uses modern interfaces without silent legacy fallback.
- [ ] A documented physical `x86_64` machine boots from NVMe, routes interrupts through APIC/MSI-X, and uses xHCI input.
- [ ] Multi-core stress preserves scheduling, memory ownership, capabilities, and storage consistency.
- [ ] Device reset, timeout, malformed DMA completion, surprise removal, suspend, and resume have repeatable tests.

## Stage 9 — Userspace graphics and product experience

**Status: deferred until Stages 6-8 establish the required contracts**

Planned work:

- a userspace window server and compositor;
- isolated shared-memory surfaces;
- virtio-gpu for the reference VM and a documented physical-GPU path;
- scalable text shaping, fonts, layout, themes, input methods, and accessibility primitives;
- clipboard and drag-and-drop capabilities;
- userspace terminal, Files, Tasks, Settings, recovery, and launcher applications;
- visual, interaction, accessibility, memory, and frame-latency regression suites.

Acceptance criteria:

- [ ] Product UI no longer executes in Ring 0.
- [ ] Applications cannot draw into or read another application's surface.
- [ ] Administrative and recovery workflows remain possible through the serial terminal.
- [ ] Keyboard-only and screen-reader-oriented reference flows pass.
- [ ] Supported resolutions, scaling factors, focus states, errors, and recovery states have visual regression coverage.

## Stage 10 — Hardened preview and daily-use qualification

**Status: planned**

This stage converts individual subsystem proofs into a supported reference product.

Acceptance criteria:

- [ ] The verified and hardened-preview release definitions in `docs/ENGINEERING_QUALITY.md` pass.
- [ ] Upgrade, rollback, recovery, and data-backup procedures are tested from prior supported versions.
- [ ] The project publishes supported hardware, known limitations, security support period, and compatibility policy.
- [ ] Long-duration stress covers memory pressure, process churn, storage faults, network faults, suspend/resume, and device reset.
- [ ] Independent reviewers can reproduce the release image and benchmark artifacts.
- [ ] No universal superiority claim appears in release material; every comparison links to reproducible evidence.

## Cross-cutting scorecard

Each release records the following. A missing measurement is reported as missing, not silently treated as zero.

| Dimension | Required evidence |
| --- | --- |
| Correctness | invariant tests, fault injection, parser corpora, cleanup accounting, repeated boots |
| Security | threat models, isolation tests, unsafe inventory, protection-bit proof, fuzz results |
| Reliability | crash and power-loss recovery, timeout/reset behavior, long-duration stress |
| Performance | boot time, idle CPU, idle memory, binary size, syscall/process latency, storage/network throughput |
| Maintainability | module ownership, public contracts, reviewable patches, warning-free builds, architecture records |
| Hardware | exact VM configuration and exact physical reference-machine reports |
| Compatibility | ABI/version policy, migration tests, rollback behavior, supported application set |
| Accessibility | keyboard operation, focus behavior, text alternatives, assistive-technology contracts |

## Benchmarking against Linux or another system

Every comparison must publish:

1. the exact GenOS commit and build profile;
2. the exact comparison-system version, configuration, services, and kernel command line;
3. identical hardware or an identical pinned virtual-machine definition;
4. the workload source and commands;
5. warm-up policy, sample count, raw results, variance, and failure count;
6. memory, CPU, storage, and network measurement method;
7. known advantages or missing features that affect the result;
8. a reproducible artifact or script in the repository.

A result applies only to that experiment. It does not imply that either operating system is universally better.

## How roadmap changes are made

A roadmap change must explain:

1. the user or developer problem;
2. why the work belongs in the proposed stage;
3. the smallest useful vertical slice;
4. success and negative-path acceptance criteria;
5. security, compatibility, performance, and maintenance costs;
6. dependencies and rollback behavior;
7. which documentation and measurements must change.

Major architectural changes should use an architecture proposal issue and an architecture decision record. Working code, clear contracts, repeatable evidence, and long-term maintainability decide priority.