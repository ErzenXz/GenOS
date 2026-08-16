# ADR-0000: Decision title

- **Status:** Proposed
- **Date:** YYYY-MM-DD
- **Decision owners:** GitHub usernames or project roles
- **Related issue:** `#NNN`
- **Related roadmap gate:** F0, Stage 6, or another exact anchor
- **Supersedes:** None
- **Superseded by:** None

## Context

Describe the concrete problem, current behavior, evidence, affected users or contributors, and why the decision is needed now.

State the constraints that materially shape the decision:

- architecture and hardware;
- privilege and authority boundaries;
- memory, CPU, I/O, and latency budgets;
- compatibility and migration commitments;
- failure, recovery, and support requirements;
- current known limitations.

## Decision

State the chosen contract precisely.

Define:

- owner of every mutable resource;
- public and internal interfaces;
- success behavior;
- denied and malformed-input behavior;
- exhaustion and out-of-memory behavior;
- timeout and cancellation behavior;
- close, exit, fault, kill, reset, rollback, and recovery behavior;
- versioning and compatibility policy;
- default path and explicit fallback path.

## Invariants

List the properties that must always hold. Each invariant should be testable or auditable.

Examples:

- one frame has one owner;
- a stale generation cannot regain authority;
- no interrupt path blocks;
- no application mapping is writable and executable;
- a failed persistent mutation leaves one valid committed generation;
- fallback hardware cannot emit the preferred-path success marker.

## Alternatives considered

### Alternative A

Describe the alternative, its benefits, costs, failure modes, and why it was not selected.

### Alternative B

Describe the alternative, its benefits, costs, failure modes, and why it was not selected.

Include “keep the current design” when that is a realistic option.

## Consequences

### Positive

- What becomes simpler, safer, faster, more testable, or more coherent?

### Negative

- What complexity, resource cost, compatibility burden, or maintenance work is accepted?

### Deferred

- What remains explicitly out of scope?

## Security analysis

Describe:

- attacker capabilities;
- trusted components;
- authority granted and denied;
- isolation boundary;
- malformed, stale, replayed, canceled, exhausted, downgrade, and rollback cases;
- unsafe code or assembly introduced or changed;
- residual risk.

## Reliability and recovery

Describe partial construction, partial I/O, power loss, timeout, reset, process termination, and restart behavior. State how ownership and externally visible state are reconciled.

## Performance and resource budgets

State expected costs and hard bounds:

- memory and queue capacity;
- CPU and interrupt work;
- copies and allocations;
- latency and throughput expectations;
- benchmark and regression method.

Do not use unsupported qualitative claims.

## Compatibility and migration

Describe affected ABI, storage, wire, package, driver, boot, configuration, and user-visible contracts.

State:

- compatibility promise;
- migration steps;
- downgrade behavior;
- rollback plan;
- how old and new versions fail when mixed.

## Verification

List the required evidence before acceptance:

- host unit tests;
- property or fuzz tests;
- QEMU success and failure phases;
- fault-injection matrix;
- long-run test;
- reference-hardware result;
- benchmark artifact;
- documentation and limitations updates.

Use exact markers, commands, metrics, and thresholds where possible.

## Implementation plan

Break the decision into reviewable commits or pull requests. Each step should leave the repository buildable and preserve a valid intermediate contract.

## References

Link relevant issues, pull requests, code, specifications, subsystem documents, benchmark artifacts, and prior ADRs.