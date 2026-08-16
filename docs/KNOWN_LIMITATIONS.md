# GenOS known limitations

This document records limitations that materially affect correctness, security, reliability, compatibility, performance, or release claims. It is not a complete bug list.

Baseline audited: GenOS 0.49 at `c26fcecfdeec64e96e3193aab016bc3356154530`.

A limitation remains open until implementation, negative tests, cleanup tests, and the relevant QEMU or hardware proof land. A milestone marked delivered elsewhere means its narrow demonstration worked. It does not override this register.

## Release status

GenOS is an **experimental developer preview**.

Do not use the current image for:

- important or irreplaceable data;
- secrets, credentials, or personal data;
- production services;
- hostile networks;
- untrusted applications or devices;
- unsupported physical hardware;
- workloads that require a maintained compatibility or security-support contract.

The immediate remediation order is defined by the [foundation correctness gate](../ROADMAP.md#immediate-priority-foundation-correctness-gate).

## Exception and interrupt coverage

The current IDT is initialized with a catch-all assembly entry that executes only `iretq`. Later initialization replaces a small set of vectors, including double fault, general protection fault, page fault, timer, keyboard, mouse, and the syscall vector.

Consequences:

- an unhandled exception without an error code may return to the same faulting instruction and loop;
- an unhandled exception with a CPU-pushed error code does not match a bare `iretq` frame;
- exceptions such as divide error, invalid opcode, invalid TSS, segment-not-present, stack fault, alignment check, and machine check do not yet have complete deliberate handling;
- unexpected external interrupts do not yet share a normalized dispatcher and policy.

Required fix: Roadmap F1.

## CPU protection and W^X

The ELF loader rejects writable-and-executable load segments. Page mapping adds the no-execute bit only when `EFER.NXE` is already enabled. The audited baseline reads that state but does not explicitly enable and verify it.

Consequences:

- firmware state can determine whether writable user data and user stacks are executable;
- the project cannot yet claim hardware-enforced system-wide W^X;
- `CR0.WP`, SMEP, and SMAP are not yet release-gated and proven by negative tests;
- kernel text, read-only data, and mutable data do not yet have a published final permission map and proof.

Required fix: Roadmap F2.

## Physical and virtual memory

The current physical allocator is a gap-safe bump allocator with a fixed 256-entry recycled-frame array.

Consequences:

- it cannot represent every reclaimed frame after the fixed recycle array is full;
- it is not a general fragmented-memory allocator;
- it does not provide ordered or contiguous allocations as a formal contract;
- allocator ownership is not yet represented by complete page-state metadata;
- recursive page-table cloning can allocate several frames before a later out-of-memory return, without a complete transactional rollback path;
- every address-space construction path has not yet been fault-injected at each allocation boundary.

Required fix: Roadmap F3.

## Single-core and shared state

GenOS is currently a single-core system. Several architecture, memory, paging, process, scheduler, and runtime states are stored in mutable globals or assume only one active kernel execution path.

Consequences:

- starting another processor would introduce races in frame allocation, current-process state, address-space state, scheduling, and cleanup;
- per-CPU current process, interrupt-local state, run queues, and TLB shootdowns do not exist yet;
- synchronization and lock-order rules are not yet a public kernel contract;
- delayed or nested interrupt behavior does not yet have a complete stress matrix.

Required fix: Roadmap F5. SMP must remain disabled until that gate passes.

## Runtime concentration and module ownership

`kernel/src/userspace.rs` currently owns or coordinates many responsibilities, including process construction, contexts, scheduling, user-copy validation, syscall work, typed handles, lifecycle, socket authority, cleanup, and boot proofs.

Consequences:

- unrelated changes touch the same large file;
- invariants are harder to review at module boundaries;
- mechanical movement and behavior changes are difficult to separate;
- future concurrency would multiply the number of mutable relationships inside one module.

The current runtime ownership document improves conceptual ownership, but source ownership still needs decomposition.

Required fix: Roadmap F4.

## Validation work in normal boot

The current boot path performs extensive lifecycle, rollback, generation-reuse, scheduler, console, handle, request-identity, storage, and networking proofs.

Consequences:

- a normal boot and a validation boot are not yet separate product policies;
- startup cost includes development stress work;
- release behavior can accidentally depend on state prepared by a probe;
- disabling one probe may change normal boot in ways the release contract does not expose.

Required fix: Roadmap F6.

## Process and ABI limits

Current userspace is intentionally bounded and does not provide general POSIX or Windows compatibility.

Notable limits include:

- a small fixed process table and one active user process at a time during the current transition model;
- small per-process typed-handle tables;
- one published endpoint per process;
- scalar endpoint messages rather than general byte streams or typed serialization;
- no capability delegation between processes;
- no general service name registry;
- fixed user image, data, stack, path, file, queue, and syscall-buffer budgets;
- no userspace heap or memory-mapping API;
- no fork, copy-on-write, shared libraries, dynamic linker, signals, threads, or mature asynchronous event API;
- no stable external application SDK or compatibility support period;
- the application ABI is experimental and can change.

These bounds are useful for current reasoning. They must become explicit resource policy rather than hidden system-wide ceilings before a broader application platform.

## Scheduling

The current scheduler is a small round-robin design for the bounded reference workload.

It does not yet provide:

- priorities or scheduling classes for userspace;
- real-time policy;
- fair scheduling across large process counts;
- multi-core load balancing;
- CPU affinity;
- general readiness polling or scalable wait queues;
- comprehensive priority-inversion handling;
- resource groups or quotas.

Existing scheduler measurements cover narrow policy and CR3-switch behavior, not end-to-end application latency or system scalability.

## Storage

The current persistent system is a bounded custom snapshot format on an MBR partition with an ATA-based reference path.

Notable limits include:

- small node, path, file, cache, and snapshot budgets;
- whole-snapshot persistence rather than a general extent or block allocator;
- no journaling filesystem, copy-on-write tree, quotas, sparse files, large files, links, permissions, timestamps, or mature metadata model;
- no encryption at rest;
- no production backup, upgrade, migration, or compatibility contract;
- no NVMe default path;
- no IOMMU-backed DMA isolation;
- power-loss and partial-I/O coverage is not yet a complete matrix across every mutation point;
- important data should not be stored on the current implementation.

The storage documentation describes the exact bounded format and recovery behavior already demonstrated. It should not be read as a general filesystem claim.

## Networking

The current network stack proves bounded IPv4 UDP and TCP vertical slices. It is not a production TCP/IP implementation.

Notable limits include:

- a small fixed number of in-flight client and passive operations;
- one bounded request and response rather than arbitrary long-lived TCP streams;
- limited simultaneous listener and accepted-client behavior;
- fixed small send, receive, and backlog budgets;
- incomplete segmentation, reassembly, dynamic windows, congestion control, RTT estimation, loss recovery, and fairness;
- polling remains part of current VirtIO progress and recovery behavior;
- no completed MSI-X interrupt-driven data path;
- no IPv6;
- no TLS, HTTPS, reviewed trust store, secure time, or certificate validation;
- no firewall, routing policy, network namespace, packet filter, or mature observability;
- hostile network exposure is unsupported.

Required next work: Roadmap Stage 5.4E, Stage 5.5, and Stage 6.

## Hardware support

The supported development target is the documented QEMU/OVMF `x86_64` reference configuration.

The project does not yet provide a supported physical-hardware matrix. Missing or incomplete areas include:

- ACPI-based discovery and power control;
- APIC/x2APIC, I/O APIC, MSI, and MSI-X as the normal interrupt path;
- SMP;
- xHCI and general USB;
- NVMe as the primary storage path;
- IOMMU and DMA isolation;
- mature PCIe reset, error, power-state, and hotplug behavior;
- audio;
- Wi-Fi;
- a production GPU driver path;
- laptop battery, thermal, suspend, resume, and low-power states;
- installer and recovery media for real machines.

Legacy PS/2, PIC/PIT, and ATA paths are development or recovery mechanisms, not the intended modern baseline.

## Security model

Typed capabilities and exact asynchronous request identity are strong experimental foundations, but GenOS does not yet provide a complete production security model.

Missing or incomplete areas include:

- complete exception and page-protection hardening;
- user and service identity;
- filesystem permissions and ownership;
- delegated capabilities with attenuation;
- application sandbox profiles;
- cryptographic entropy and a CSPRNG policy;
- TLS and certificate validation;
- signed packages and updates;
- rollback protection;
- a versioned trust store and secure time;
- secure or measured boot policy;
- secrets storage;
- IOMMU-backed isolation for DMA-capable devices;
- a published threat model for every trusted boundary;
- security-maintenance releases and response service-level objectives.

Do not treat Rust as proof that the kernel is memory-safe. Architecture code, page tables, device access, DMA, raw pointers, inline assembly, and interrupt entry still rely on unsafe invariants.

## Graphics and desktop

The active product path is serial-first. The older framebuffer desktop remains development code and is not the quality target.

Missing or incomplete areas include:

- a userspace window server and compositor;
- isolated graphics surfaces;
- scalable typography and text shaping;
- stable input methods, clipboard, drag-and-drop, and accessibility contracts;
- a physical GPU strategy;
- visual, interaction, accessibility, memory, and latency regression suites;
- general application lifecycle and package integration.

Product UI must not become a reason to move service ownership or application logic back into Ring 0.

## Build, CI, and release process

At the audited baseline, the latest `main` CI run stopped during Clippy, so workspace tests and QEMU boot did not execute. This PR fixes the two reported task-metrics lints, but the branch is not considered verified until the complete pull-request workflow passes.

The project still needs:

- a declared and tested toolchain policy;
- complete debug and release build coverage;
- separate fast, scheduled, and hardware lanes;
- retained fuzz corpora;
- generated unsafe inventory;
- published image hashes and reproducibility metadata;
- branch protection that requires all release-relevant checks;
- release artifacts tied to supported configurations and known limitations.

Required fix: Roadmap F0, F6, and F7.

## Performance claims

GenOS has narrow scheduler and address-space-switch measurements. It does not yet have a complete public performance baseline.

Do not claim that GenOS is faster, lighter, safer, or better than another operating system without the experiment required by [the engineering quality plan](ENGINEERING_QUALITY.md#comparison-with-linux-windows-macos-bsd-or-another-os).

Missing measurements include complete boot time, idle memory, idle CPU, binary-size history, syscall latency, process lifecycle, storage recovery, network throughput and loss recovery, and later graphics latency under pinned configurations.

## Updating this register

A pull request must update this file when it:

- fixes or narrows a listed limitation;
- introduces a new system-wide bound;
- changes supported hardware, security, compatibility, storage, or network behavior;
- changes a release-level claim;
- discovers a failure that affects user or contributor expectations.

Remove a limitation only in the same change that adds the required implementation and evidence, or in a follow-up change that links directly to already merged proof.