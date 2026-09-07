# Security policy

GenOS is an experimental operating system. It is not yet appropriate for sensitive data, hostile workloads, production services, or untrusted physical devices.

Security reports are valuable now because early findings can change boundaries before they become compatibility commitments.

## Reporting a vulnerability

Do **not** open a public issue for a suspected vulnerability.

Use GitHub's private vulnerability reporting flow:

1. Open the repository's **Security** tab.
2. Choose **Report a vulnerability**.
3. Include the affected commit, configuration, subsystem, reproduction steps, expected behavior, observed behavior, impact, and any suggested mitigation.

Helpful reports include:

- a minimal image, packet, ELF file, syscall sequence, storage state, or device state that reproduces the problem;
- serial output and the last reliable marker;
- whether the failure crosses a Ring 3/Ring 0, process, capability, storage, network, or DMA boundary;
- whether reboot, reset, cancellation, timeout, or recovery changes the outcome;
- whether the issue is deterministic;
- any proof that a stale PID, generation, handle, request, mapping, or completion regains authority.

The project aims to acknowledge a complete report within seven days. Because GenOS is currently maintained as an experimental project, remediation time depends on impact and maintainer availability. Disclosure timing will be coordinated with the reporter.

## Supported versions

Only the current `main` branch is supported during the experimental stage.

There are currently:

- no stable security-maintenance releases;
- no long-term support branch;
- no guaranteed patch deadline;
- no production deployment recommendation;
- no backward-compatible security-update contract.

A future hardened preview must publish supported versions, support periods, update and rollback policy, image hashes, signing policy, and known limitations before this section can claim more.

## Current security foundations

The current experimental system includes useful security-oriented mechanisms:

- separate Ring 3 address spaces;
- validated syscall arguments and user-copy ranges;
- typed, process-local, rights-bearing handles;
- generation checks for stale capabilities;
- exact request identities for deferred work;
- cleanup on close, exit, fault, kill, cancellation, and reap;
- bounded parser, queue, path, file, process, handle, and network resources;
- rejection of writable-and-executable ELF segments;
- deterministic negative tests for selected forged, wrong-type, wrong-rights, stale, canceled, and replayed operations.

These foundations do not form a complete production security model.

## Current security limitations

The material limitations are tracked in [docs/KNOWN_LIMITATIONS.md](docs/KNOWN_LIMITATIONS.md). Release-blocking areas include:

- incomplete exception and unexpected-interrupt coverage;
- CPU page-protection features that are not yet explicitly enabled and proven by the kernel;
- a fixed-capacity physical-frame recycle path and incomplete allocation rollback;
- single-core assumptions and unsynchronized mutable global state;
- concentrated runtime ownership in large kernel modules;
- no user or service identity, filesystem permissions, or general capability delegation;
- no cryptographic entropy policy, TLS, trust store, secure time, signed packages, or signed updates;
- no IOMMU-backed DMA isolation;
- no supported physical-hardware or hostile-network threat model;
- no stable compatibility or security-support contract.

Rust reduces broad classes of memory errors, but it does not prove the kernel safe. Page tables, raw pointers, inline assembly, interrupt entry, firmware data, MMIO, port I/O, DMA, device queues, and unsafe aliasing still depend on manually maintained invariants.

## High-priority report classes

Reports are especially valuable when they involve:

- privilege escalation or Ring 3 to Ring 0 execution;
- cross-process memory or capability access;
- executable writable memory or bypass of page protections;
- malformed exception frames, stack corruption, triple fault, or silent fault loops;
- stale handle, PID reuse, generation wrap, request replay, or canceled-work mutation;
- frame aliasing, double free, use-after-free, leak, or page-table rollback failure;
- persistent data corruption, rollback confusion, repair-tool data loss, or unsafe read-write recovery;
- malformed packet or device input causing memory corruption or unbounded work;
- DMA outside an owned buffer;
- package, update, trust, or downgrade bypass when those systems land;
- a CI or release-integrity failure that allows an unverified image to appear trusted.

## Security expectations for changes

Security-sensitive pull requests should state:

1. attacker capability and protected asset;
2. trusted boundary and authority model;
3. success and denial behavior;
4. malformed, stale, replayed, canceled, exhausted, timeout, and rollback cases;
5. unsafe code and assembly changed;
6. positive and negative tests;
7. compatibility and downgrade effects;
8. remaining limitations.

The full engineering expectations are in [docs/ENGINEERING_QUALITY.md](docs/ENGINEERING_QUALITY.md) and [CONTRIBUTING.md](CONTRIBUTING.md).

## Coordinated disclosure

Before public disclosure, the maintainer and reporter should agree on:

- affected commits and configurations;
- severity and realistic impact;
- fix or mitigation;
- regression test and evidence;
- release or commit containing the fix;
- credit;
- disclosure date.

A fix should update the limitations register when it closes or narrows a listed gap. Security claims should describe the exact corrected boundary and evidence rather than implying that the whole operating system became secure.