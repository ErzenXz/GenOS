## What changed

Describe the behavior or contract changed by this pull request.

## Why

Explain the user, contributor, reliability, or performance problem.

## Verification

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy -p kernel --lib -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `make test`
- [ ] QEMU screenshot attached when the framebuffer or desktop changed

## Risk and recovery

Describe boot, memory-safety, compatibility, security, or performance risks and how the change can be reverted or diagnosed.

## Scope

- [ ] The change is focused and contains no unrelated cleanup.
- [ ] Unsafe invariants are documented.
- [ ] Public behavior and roadmap documentation are updated where needed.
