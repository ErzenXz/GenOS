# Contributing to GenOS

Thank you for helping build GenOS. This is an early operating-system project, so small changes can affect boot, memory safety, interrupt handling, and every application at once. The contribution process is designed to keep `main` understandable and bootable.

## Before you start

- Read the [README](README.md) and [roadmap](ROADMAP.md).
- Search existing issues before opening a new one.
- Discuss large architectural changes before implementing them.
- Keep proposals tied to a concrete user, developer, reliability, or performance outcome.

Small fixes, tests, documentation improvements, and well-scoped usability work can go directly to a pull request.

## Development setup

Install the tools listed in the README, then verify your environment:

```sh
rustup target add x86_64-unknown-uefi x86_64-unknown-none
make build
make test
```

Useful focused checks:

```sh
cargo fmt --all -- --check
cargo clippy -p kernel --lib -- -D warnings
cargo test --workspace
```

## Pull-request expectations

A good GenOS pull request:

- solves one clearly described problem;
- explains observable behavior before and after the change;
- includes tests or a reason automated coverage is not possible;
- keeps boot-critical serial diagnostics intact;
- updates documentation when a command, contract, or limitation changes;
- avoids unrelated formatting or refactoring;
- passes `make test` from a clean checkout.

For visual changes, include a QEMU screenshot. For performance changes, include the command, environment, baseline, result, and enough detail to reproduce the measurement.

## Kernel constraints

- The kernel library and binary must remain `no_std`.
- Heap allocation is not assumed unless the relevant subsystem explicitly provides it.
- Bounded storage is preferred for early kernel queues and registries.
- Every `unsafe` block must have a narrow scope and a documented invariant.
- Interrupt handlers must avoid unbounded or blocking work.
- Hardware input and boot data must be treated as untrusted.
- Cross-boundary structures belong in the versioned ABI crate.

## Commit style

Use short imperative commit subjects:

```text
Add extended keyboard decoding
Keep focused windows above background apps
Document Stage 2 process isolation criteria
```

Separate mechanical cleanup from behavioral changes when practical.

## Proposing a major subsystem

Open an issue before implementing scheduling, userspace, storage formats, networking contracts, security boundaries, package formats, or a public application ABI. Include:

1. the problem and intended user;
2. the proposed contract;
3. alternatives considered;
4. failure and recovery behavior;
5. security and performance implications;
6. the smallest testable vertical slice;
7. migration or compatibility implications.

## Review culture

Reviews should be direct, respectful, and focused on the system. Explain why a change is risky or valuable. Prefer evidence and a suggested path forward over vague approval or rejection.

By participating, you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
