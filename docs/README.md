# GenOS documentation

GenOS documentation separates current contracts, future sequence, material limitations, engineering evidence, and durable architecture decisions.

## Start here

1. [Project overview](../README.md)
2. [Roadmap and acceptance gates](../ROADMAP.md)
3. [Known limitations](KNOWN_LIMITATIONS.md)
4. [Engineering quality plan](ENGINEERING_QUALITY.md)
5. [Contribution guide](../CONTRIBUTING.md)
6. [Security policy](../SECURITY.md)

## Current subsystem contracts

These documents describe the exact bounded behavior demonstrated by the current experimental baseline. They do not imply production readiness and do not override the limitations register.

- [Userspace boundary and ABI](USERSPACE.md)
- [Runtime ownership and coordination](RUNTIME.md)
- [Storage format and recovery](STORAGE.md)
- [Networking contracts](NETWORKING.md)

## Architecture decisions

- [ADR index and workflow](adr/README.md)
- [ADR template](adr/0000-template.md)

Use an ADR for a durable contract such as exception frames, allocation, scheduling, concurrency, syscalls, capabilities, storage formats, socket semantics, driver boundaries, packages, updates, trust, compatibility, or graphics isolation.

## Document responsibilities

| Document | Responsibility |
| --- | --- |
| `README.md` | concise current project identity, supported development path, build instructions, and status |
| `ROADMAP.md` | ordering, dependencies, stage status, and testable acceptance criteria |
| `KNOWN_LIMITATIONS.md` | material correctness, security, reliability, compatibility, hardware, and release gaps |
| `ENGINEERING_QUALITY.md` | release levels, invariants, CI, fuzzing, fault injection, benchmarks, and definition of done |
| subsystem document | exact current ownership, ABI, state, bounds, success, failure, and cleanup contract |
| ADR | reasoning and consequences of one durable architecture decision |
| `CONTRIBUTING.md` | review and delivery rules |
| `SECURITY.md` | reporting, supported versions, and disclosure policy |

## Updating documentation

Update the same pull request when code changes:

- user-visible commands or behavior;
- ABI, storage, wire, driver, or lifecycle contracts;
- authority, ownership, cleanup, or synchronization;
- supported hardware or release level;
- a roadmap gate or known limitation;
- a benchmark's meaning;
- an unsafe assumption;
- migration, downgrade, rollback, or recovery behavior.

A serial marker proves only the condition it names. A screenshot proves only the displayed state. Documentation should state the smallest claim supported by the evidence.