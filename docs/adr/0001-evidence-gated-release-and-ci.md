# ADR-0001: Evidence-gated release and CI policy

- **Status:** Proposed
- **Date:** 2026-08-17
- **Decision owners:** `@ErzenXz`
- **Related issue:** PR #4
- **Related roadmap gate:** F0, F6, F7
- **Supersedes:** None
- **Superseded by:** None

## Workflow exception

This ADR and the architecture-proposal workflow land together in PR #4. No preceding proposal form existed. The maintainer therefore approved review of this initial policy decision in the pull request itself rather than creating a retroactive proposal issue. Future durable decisions follow the normal proposal-first workflow.

## Context

GenOS has delivered several bounded operating-system demonstrations, but the previous roadmap and CI presentation made it too easy to confuse a successful marker, screenshot, or narrow smoke test with a release-level guarantee. The `main` workflow also stopped at strict kernel-binary Clippy before workspace tests and QEMU ran.

The project needs one durable policy for:

- separating current evidence from target release gates;
- defining which checks are enforced now and which remain planned;
- preventing lint failures from hiding test and boot results;
- retaining enough metadata and serial output to inspect a CI result;
- making operating-system and performance claims reproducible and scoped.

## Decision

GenOS will use an evidence-gated release model.

1. `ROADMAP.md` owns sequencing and acceptance gates.
2. `docs/KNOWN_LIMITATIONS.md` owns material gaps in the audited baseline.
3. `docs/ENGINEERING_QUALITY.md` owns release levels, target invariants, evidence formats, and target CI lanes.
4. `.github/workflows/ci.yml` is the source of truth for checks currently enforced in pull requests.
5. Documentation must label target checks as targets until the repository actually runs and enforces them.
6. Rust builds use the repository-pinned toolchain in `rust-toolchain.toml`.
7. Pull-request CI separates static analysis, host tests, release-profile compilation, documentation checks, and QEMU system tests so one category cannot hide the result of another.
8. Every shipped Rust target is checked with warnings denied. Deliberately retained dormant paths may use narrow, locally explained lint exceptions until their roadmap gate removes or isolates them.
9. QEMU jobs retain phase-specific serial logs and a manifest containing the commit, tool versions, VM dependencies, and image hash.
10. A green pull request proves only the checks currently implemented. It does not close F0-F7 or raise the release level by itself.

## Invariants

- A current claim names the exact evidence that exists now.
- A target requirement is not described as currently enforced.
- A strict Clippy result covers the full shipped kernel binary, not only the library.
- Static-analysis failure does not prevent host tests or QEMU from reporting their own results.
- A successful system-test run retains inspectable serial evidence and environment metadata.
- A release-level label changes only when every gate required by that level is complete.
- Comparison claims identify the workload, configuration, baseline, raw results, variance, failures, and missing features.

## Alternatives considered

### Keep one sequential CI job

This is simpler, but an early formatting or lint failure hides whether host tests, release compilation, or QEMU also fail. It does not satisfy F0's diagnostic goal.

### Disable strict kernel-binary Clippy

This allows the workflow to reach tests, but it leaves the shipped binary outside the warning policy. The selected approach instead fixes active diagnostics and documents narrow retained-code exceptions.

### Treat the quality plan as immediately enforced

This makes the document sound stronger, but it is false until fuzzing, unsafe inventory, scheduled repetition, branch protection, and other lanes exist. The selected approach records current and target states separately.

## Consequences

### Positive

- CI failures become easier to diagnose.
- The full kernel image receives strict static analysis.
- Contributors can run the same commands that CI runs.
- Reviewers can inspect serial evidence from successful and failed QEMU jobs.
- Roadmap and release language become harder to overstate.

### Negative

- CI uses more parallel jobs and repeats some dependency setup.
- Retained serial logs consume artifact storage.
- Narrow lint exceptions remain until dormant graphical and recovery paths are isolated or removed.
- F0 still remains open after this decision because several target lanes and enforcement mechanisms are not implemented.

### Deferred

- fuzz targets and retained corpora;
- generated unsafe-code inventory;
- architecture-boundary enforcement beyond existing source tests;
- scheduled long-run and hardware lanes;
- branch protection and required-check administration;
- reproducible release image publication and signing.

## Security analysis

This policy does not make the kernel secure. It reduces the chance that an untested or partially tested image appears verified. The remaining exception-entry, page-protection, allocator, concurrency, DMA, cryptographic, and update gaps stay release-blocking in `docs/KNOWN_LIMITATIONS.md`.

The workflow uses read-only repository permissions. Artifact uploads contain build metadata and serial output, not repository credentials or signing material.

## Reliability and recovery

Independent jobs preserve diagnostic information when another job fails. QEMU logs are uploaded with `if: always()` so timeouts and partial boots remain inspectable. The manifest records enough tool and image information to reproduce the reference configuration more accurately.

## Performance and resource budgets

This decision makes no performance claim about GenOS. CI duration and artifact size are operational costs. Serial logs and one text manifest are retained; the full disk image is not uploaded in the pull-request lane.

## Compatibility and migration

No kernel ABI, storage format, wire protocol, or user-visible runtime behavior changes. Contributor commands and CI check names change. Existing pull requests must follow the current commands in `.github/workflows/ci.yml` and may reference the target lanes as future gates.

Rollback consists of reverting this ADR and the associated workflow/documentation change. Reverting must not reintroduce unsupported release claims.

## Verification

Before acceptance:

- formatting passes;
- strict Clippy passes for the ABI, tooling, bootloader, kernel library, complete kernel binary, runtime, init, and shell targets;
- workspace tests pass;
- release-profile target checks pass;
- Markdown link validation passes;
- the multi-phase QEMU suite passes;
- the QEMU job uploads serial logs and the CI manifest;
- the roadmap, quality plan, limitations register, contribution guide, README, PR template, and PR body use consistent current-versus-target language.

## Implementation plan

1. Clear strict kernel-binary diagnostics without changing runtime behavior.
2. Pin the supported Rust toolchain.
3. Split CI into independent jobs and retain QEMU evidence.
4. Add Markdown link validation.
5. Align roadmap, quality, limitations, contribution, README, and PR language.
6. Keep F0-F7 open until their remaining criteria are implemented and merged.

## References

- `ROADMAP.md`
- `docs/ENGINEERING_QUALITY.md`
- `docs/KNOWN_LIMITATIONS.md`
- `.github/workflows/ci.yml`
- `CONTRIBUTING.md`
