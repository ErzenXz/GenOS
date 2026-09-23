# ADR-0009: Keep the native ABI capability-explicit and versioned

- **Status:** Proposed; records the implemented ABI 18 direction, not a stable SDK promise.
- **Date:** 2026-09-23
- **Decision owners:** userspace/runtime maintainers
- **Related roadmap:** F4.5, C1, F7.3
- **Supersedes:** None

## Decision

GenOS runs separately linked static x86_64 ELF applications at Ring 3. The
kernel and userspace runtime share `genos_abi` layouts and numbers, currently
ABI 18 and image layout 2. Applications enter through the bounded `int 0x80`
interface. A process receives explicit typed console, file, namespace,
lifecycle, endpoint and socket handles rather than authority from a path,
PID or raw address alone. The kernel validates each user range and handle
kind/rights/owner before publishing a request; copied request bytes outlive
the caller's temporary buffer. Completion rechecks request and process
incarnation. Close, normal exit, fault and kill revoke remaining authority.

The current image window has one RW ABI data page, at most eight RX executable
pages, four stack pages and an unmapped stack guard. `SHELL.ELF` compiles with
a size-oriented package profile to remain within that **unchanged** window;
the build must link, not merely pass Clippy. The 80-byte console text bound,
128-byte file/socket transfer units and typed structures are part of this
experimental ABI. The bounded editor does not turn the console handle into
general stdin/stdout streams.

## Why this trade-off

A POSIX-like ambient namespace and `fork` would be familiar, but would add
inheritance, copy-on-write, descriptor and cancellation semantics before
GenOS can account for its current handles and frames. The selected explicit
interface makes a small static application possible now and exposes denial
and cleanup as testable operations. It also limits application breadth:
general named spawn, argument/environment passing, heap growth, streams and
service supervision remain C1 work, not hidden semantics of `run init`.

## Failure and compatibility

Wrong-kind/stale/forged handles, invalid buffers, unsupported ELF layouts,
full tables and exhausted frame quotas fail without a runnable child or
partial capability. A request canceled after admission must not apply a stale
completion to a reused process slot. ABI additions that change a structure's
meaning or size require a versioned record or feature negotiation; adding
key codes 8–13 to the existing fixed event layout is a compatible extension.
The package version alone does not negotiate ABI compatibility.

The downgrade path is a matched old kernel, runtime, applications and data
image, not an old app mixed with an arbitrary new kernel. Stop sessions,
close/revoke handles, retain both complete bundles, and test unsupported
version errors before activation. A running process, socket or console line
does not migrate across reboot. The [compatibility inventory](../COMPATIBILITY.md)
records each public ABI/storage boundary and its rollback limits.

## Evidence and limits

`cargo test -p genos_abi`, `cargo test -p genos-shell --lib`,
`cargo xtask test-sdk`, the clean full suite and the ten-case
[serial editing proof](../../tools/test_terminal_editing.py) exercise actual
layouts, external linking, Ring 3 execution and typed denial/cleanup paths.
The [userspace contract](../USERSPACE.md) and [shared ABI](../../crates/abi/src/lib.rs)
are the implementation references. This ADR does not establish a stable
binary support period, hostile-application security audit, dynamic loader or
general stream/launch contract.
