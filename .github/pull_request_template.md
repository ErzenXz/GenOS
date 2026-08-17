## Problem

What concrete user, developer, correctness, security, reliability, performance, or maintainability problem does this pull request solve?

## Contract before and after

Describe observable behavior before the change and the exact behavior after it. Include authority, ownership, limits, compatibility, and failure semantics where relevant.

## Scope

- [ ] This pull request contains one logical change or one inseparable vertical slice.
- [ ] Mechanical movement is separated from behavior where practical.
- [ ] Every commit is buildable and understandable.
- [ ] Unrelated formatting, renaming, dependency updates, and cleanup are excluded.
- [ ] A larger-than-usual change explains why it cannot be split safely.

## Verification

Commands run:

- [ ] `cargo fmt --all -- --check`
- [ ] `python3 tools/check_docs.py`
- [ ] `cargo clippy -p genos_abi -- -D warnings`
- [ ] `cargo clippy -p xtask -- -D warnings`
- [ ] `cargo clippy -p bootloader --target x86_64-unknown-uefi -- -D warnings`
- [ ] `cargo clippy -p kernel --lib -- -D warnings`
- [ ] `cargo clippy -p kernel --bin kernel --target x86_64-unknown-none -- -D warnings`
- [ ] `cargo clippy -p genos-user-runtime --target x86_64-unknown-none -- -D warnings`
- [ ] `cargo clippy -p genos-init --profile userspace --target x86_64-unknown-none -- -D warnings`
- [ ] `cargo clippy -p genos-shell --profile userspace --target x86_64-unknown-none -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cargo check -p bootloader --release --target x86_64-unknown-uefi`
- [ ] `cargo check -p kernel --release --target x86_64-unknown-none`
- [ ] `cargo check -p genos-user-runtime --profile userspace --target x86_64-unknown-none`
- [ ] `cargo check -p genos-init --profile userspace --target x86_64-unknown-none`
- [ ] `cargo check -p genos-shell --profile userspace --target x86_64-unknown-none`
- [ ] `make test`

Additional evidence:

- [ ] Success path covered.
- [ ] Malformed or denied input covered.
- [ ] Cleanup and resource accounting covered.
- [ ] Exhaustion or out-of-memory behavior covered where relevant.
- [ ] Timeout, cancellation, close, exit, fault, kill, reset, and rollback covered where relevant.
- [ ] Exact serial markers or artifacts are listed below.
- [ ] A visual screenshot is attached when visual behavior changed.

```text
Paste commands, markers, benchmark artifact paths, or a compact result summary here.
```

Do not mark the change verified when a required later stage was skipped because an earlier command failed. The workflow file defines checks enforced now; target lanes in the quality plan remain unverified until implemented.

## Risk classification

Check every affected class:

- [ ] boot or firmware contract
- [ ] exception, interrupt, assembly, or privilege transition
- [ ] physical memory, paging, mapping permissions, or user copy
- [ ] process, scheduler, syscall, capability, or lifecycle
- [ ] storage format, block I/O, cache, persistence, repair, or recovery
- [ ] network protocol, socket, device queue, DMA, timeout, or reset
- [ ] public ABI, format, package, update, or compatibility
- [ ] security or trust boundary
- [ ] performance-sensitive path
- [ ] presentation-only change
- [ ] documentation or tooling only

For each checked kernel or system class, describe the most serious plausible failure and how the tests detect or contain it.

## Authority, ownership, and cleanup

Who owns each new or changed resource before, during, and after the operation?

Explain how the change behaves under:

- success;
- denied authority;
- malformed input;
- partial construction;
- out of memory or queue capacity;
- timeout or cancellation;
- process exit, fault, kill, and reap;
- device reset or I/O failure;
- replay or stale identity.

Use “not applicable” only when the operation cannot encounter the case.

## Unsafe code and assembly

- [ ] No `unsafe` code or assembly changed.
- [ ] Every changed `unsafe` block has a local safety explanation.
- [ ] Assembly documents registers, clobbers, stack shape, privilege level, interrupt state, error-code shape, and return behavior.
- [ ] The changed unsafe locations and invariants are listed below.

```text
List file:function or explain why this section is not applicable.
```

## Performance

- [ ] No material performance claim or normal-path performance change.
- [ ] The exact before/after commits, environment, workload, commands, raw samples, variance, failures, and correctness checks are attached.
- [ ] Any comparison with another operating system follows `docs/ENGINEERING_QUALITY.md` and states feature differences.

## Compatibility, migration, and rollback

Describe ABI, on-disk, wire, package, driver, boot, or user-visible compatibility impact. Explain migration and how to revert or recover safely.

## Documentation and project status

- [ ] README updated when user or contributor expectations changed.
- [ ] ROADMAP updated when stage order or acceptance changed.
- [ ] `docs/KNOWN_LIMITATIONS.md` updated when a limitation changed.
- [ ] Relevant subsystem document updated.
- [ ] ADR added or superseded for a durable architecture decision.
- [ ] Security policy updated when support or disclosure expectations changed.
- [ ] No documentation update is needed, with the reason stated below.

## Remaining limitations

What does this change deliberately not solve? Link follow-up issues or roadmap gates where available.