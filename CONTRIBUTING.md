# Contributing to GenOS

GenOS is an early operating-system project. A small change can affect boot, privilege transitions, memory ownership, storage recovery, network state, or every application at once. The contribution process keeps `main` understandable, testable, and bootable.

## Before you start

Read:

- [README.md](README.md);
- [ROADMAP.md](ROADMAP.md);
- [docs/ENGINEERING_QUALITY.md](docs/ENGINEERING_QUALITY.md);
- [docs/KNOWN_LIMITATIONS.md](docs/KNOWN_LIMITATIONS.md);
- the relevant subsystem document under `docs/`.

Search open issues and pull requests before starting. Use the architecture proposal issue form for changes to interrupts, paging, allocation, scheduling, process authority, public ABI, storage formats, network contracts, drivers, packages, updates, or security boundaries.

Small fixes, tests, documentation, build improvements, and tightly scoped usability changes can go directly to a pull request.

## Development setup

Install the dependencies listed in the README, then verify the checkout:

```sh
rustup target add x86_64-unknown-uefi x86_64-unknown-none
make build
make test
```

Useful focused checks:

```sh
cargo fmt --all -- --check
cargo clippy -p kernel --lib -- -D warnings
cargo clippy -p kernel --bin kernel --target x86_64-unknown-none -- -D warnings
cargo clippy -p genos-user-runtime --target x86_64-unknown-none -- -D warnings
cargo clippy -p genos-init --profile userspace --target x86_64-unknown-none -- -D warnings
cargo clippy -p genos-shell --profile userspace --target x86_64-unknown-none -- -D warnings
cargo test --workspace
```

Do not describe a change as verified when a required later stage was skipped because an earlier command failed.

## Pull-request scope

A good pull request:

- solves one clearly described problem or establishes one contract;
- explains behavior before and after the change;
- separates mechanical movement from behavior when practical;
- keeps each commit buildable and understandable;
- includes positive, negative, cleanup, exhaustion, timeout, cancellation, and rollback tests where relevant;
- contains no unrelated formatting, renaming, dependency updates, or refactoring;
- updates public documentation and limitations when behavior changes;
- passes the complete required workflow from a clean checkout.

Roughly 500 changed non-generated lines is a reviewability signal, not a hard limit. A larger change must explain why splitting it would create an invalid or untestable intermediate state. Generated files, retained fuzz inputs, and deliberate mechanical moves should be identified separately.

Do not hide an architectural decision inside a feature patch.

## Required evidence by risk

### Boot, interrupt, or architecture changes

Include:

- the exact entry and return frame;
- privilege level and interrupt-state assumptions;
- register and stack ownership;
- error-code behavior;
- unexpected-vector and kernel-fault behavior;
- QEMU serial proof for success and failure paths.

### Memory and paging changes

Include:

- ownership before and after each operation;
- zeroing and permission policy;
- out-of-memory behavior at each allocation boundary;
- rollback and frame-count proof;
- double-free, aliasing, reserved-memory, and stale-mapping tests;
- TLB behavior when mappings change.

### Process, capability, syscall, or runtime changes

Include:

- the authority granted and who can grant it;
- typed-handle, rights, generation, owner, and stale-value behavior;
- copy-in and copy-out validation;
- exit, fault, kill, close, timeout, cancellation, replay, and reap behavior;
- exact request identity and cleanup accounting;
- ABI compatibility or migration notes.

### Storage changes

Include:

- on-disk format and version impact;
- atomicity boundaries;
- partial read, partial write, timeout, reset, torn update, and corruption behavior;
- recovery and read-only policy;
- migration and rollback plan;
- host inspection or repair-tool changes.

### Network or driver changes

Include:

- packet, descriptor, DMA, queue, and ownership validation;
- timeout, loss, duplication, reordering, reset, saturation, and cancellation behavior;
- interrupt versus polling policy;
- resource budgets and fairness;
- negative tests for malformed device and wire input;
- performance evidence when the change affects the normal data path.

### Security changes

Include:

- threat and attacker capability;
- trusted boundary and authority being protected;
- negative and bypass tests;
- unsafe code touched;
- downgrade and rollback behavior;
- disclosure or compatibility implications.

### Performance changes

Include:

- hypothesis and metric;
- exact before and after commits;
- compiler, profile, CPU, firmware, QEMU, and device configuration;
- workload and commands;
- warm-up and cache policy;
- raw samples, sample count, summary, variance, and failures;
- correctness tests run against both builds.

## Kernel constraints

- The kernel library and binary remain `no_std`.
- Heap allocation is not assumed unless the subsystem explicitly provides and bounds it.
- Hardware, firmware, files, packets, and userspace memory are untrusted input.
- Interrupt handlers perform bounded, non-blocking work.
- Shared mutable state has one owner or an explicit synchronization mechanism.
- Single-core assumptions must be asserted and documented until SMP support exists.
- Construction and mutation are transactional. Failure returns resources to their prior owners.
- Cross-boundary structures belong in a versioned ABI or format contract.
- Fallback hardware paths are explicit and cannot emit success markers for the preferred path.
- User, kernel, executable, writable, and device mappings follow documented permission policy.

## Unsafe code and assembly

Every `unsafe` block requires a nearby safety comment that explains:

1. the invariant;
2. who established it;
3. validated input and bounds;
4. aliasing, lifetime, privilege, DMA, or CPU assumptions;
5. synchronization or interrupt-state protection;
6. failure containment.

Inline assembly must document register inputs, outputs, clobbers, stack shape, privilege transition, interrupt state, error-code shape, and return behavior.

A pull request that adds or changes unsafe code must identify each affected block in the PR description. Do not use broad `allow` attributes to hide a warning without explaining why the warning does not apply.

## Tests and boot markers

Use the lowest test layer that can prove the contract, then add a system test when hardware or privilege boundaries matter.

- Pure parsers and state machines should have host tests.
- Malformed-input surfaces should have property or fuzz tests.
- Paging, interrupts, DMA, process transitions, storage commits, and device behavior need QEMU or hardware proof.
- Every fixed crash, corruption, isolation failure, or stale-authority case keeps a regression input.
- Boot-critical paths use exact serial markers with timeouts.
- Validation-only markers must not be required by release boot.

A screenshot is evidence only for a visual result. It does not prove memory safety, isolation, cleanup, or performance.

## Documentation

Update the same pull request when a change affects:

- commands or user-visible behavior;
- ABI, format, driver, or lifecycle contracts;
- a roadmap acceptance criterion;
- a known limitation;
- supported hardware or release level;
- benchmark meaning;
- unsafe assumptions or architecture ownership.

Subsystem documentation describes the exact current contract. The roadmap describes sequence and acceptance. The limitations register describes material gaps. An architecture decision record explains durable choices.

## Architecture proposals and ADRs

Before implementing a major subsystem or durable contract, open an architecture proposal containing:

1. problem and intended user or developer;
2. current behavior and evidence;
3. proposed contract and ownership;
4. alternatives considered;
5. failure, cleanup, timeout, and recovery behavior;
6. security, compatibility, performance, and maintenance impact;
7. smallest testable vertical slice;
8. migration and rollback;
9. acceptance criteria.

When the decision is accepted, add an ADR from [`docs/adr/0000-template.md`](docs/adr/0000-template.md). Update an older ADR by superseding it, not rewriting history without explanation.

## Commit style

Use short imperative subjects:

```text
Normalize x86 exception frames
Roll back failed address-space construction
Split process handles from context switching
Document the verified reference-build gate
```

Keep one logical change per commit. A reviewable series might separate:

1. tests that expose the problem;
2. implementation;
3. mechanical module movement;
4. documentation and migration.

Do not publish one milestone commit containing several unrelated subsystems when each can stand alone.

## Review culture

Reviews should be direct, specific, respectful, and evidence-based.

A useful review comment identifies:

- the violated or missing invariant;
- a concrete failure sequence;
- the affected authority or resource owner;
- the test or documentation needed;
- a smaller or safer implementation path when available.

Approval means the reviewer understands the changed contract and evidence. It does not mean the project has no remaining limitations.

By participating, you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md). Report suspected vulnerabilities through [SECURITY.md](SECURITY.md), not a public issue.