# Architecture decision records

Architecture decision records capture durable GenOS contracts and the reasoning behind them. They complement code, tests, subsystem documentation, the roadmap, and the limitations register.

Use an ADR for decisions that affect more than one implementation detail or are costly to reverse, including:

- boot and firmware contracts;
- exception-frame and privilege-transition layout;
- physical or virtual memory policy;
- scheduler, preemption, synchronization, and SMP design;
- process identity, capabilities, syscalls, and application ABI;
- filesystem or storage format;
- network socket and queue semantics;
- hardware-driver and DMA boundaries;
- package, update, trust, compatibility, or release policy;
- graphics and application isolation.

## Workflow

1. Open an architecture proposal issue.
2. Define the problem, current evidence, smallest useful slice, alternatives, failure paths, and acceptance criteria.
3. Discuss the contract before committing to a large implementation.
4. Copy [`0000-template.md`](0000-template.md) to the next four-digit number and a short lowercase title, for example `0001-x86-exception-frames.md`.
5. Set the ADR status to `Proposed` in the pull request.
6. Change the status to `Accepted` when the decision is merged.
7. If a later decision replaces it, keep the original file and mark it `Superseded by ADR-NNNN`.

Do not silently rewrite an accepted ADR to make history appear cleaner. Small corrections are acceptable when they do not change the decision. A material change should use a new ADR.

## Status values

- **Proposed:** under review and not yet the project contract.
- **Accepted:** the current project contract.
- **Rejected:** considered and deliberately not adopted.
- **Superseded:** replaced by a newer ADR.
- **Deprecated:** still present for compatibility but no longer preferred.

## Index

| ADR | Title | Status |
| --- | --- | --- |
| 0000 | Template | Template |
| 0001 | Evidence-gated release and CI policy | Proposed |

Add accepted ADRs to this index in the same pull request.