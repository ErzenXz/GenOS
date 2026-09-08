# Dependable storage, applications, terminal, and distribution

Research date: 2026-09-08. Scope: decisions for a stable reference-VM kernel and useful serial console before graphical UI. This note records primary-source facts separately from proposed GenOS choices. Recommendations are design proposals, not implemented features or evidence that the current kernel is safe.

## Current GenOS evidence

The checked-in [storage contract](../STORAGE.md) describes two 20 KiB GFS2 snapshot slots, synchronous mutation commits, an ATA cache flush, host inspection/repair, read-only recovery, and preservation of the normal user image during tests. Its namespace remains limited to 32 nodes, 64-byte paths, and 512-byte files. Those are meaningful mechanisms, but the old Stage 4 completion establishes the bounded format's original acceptance scope; it does not establish general-purpose storage readiness.

The [terminal guide](../TERMINAL.md) identifies a Ring 3 serial shell without general launch, working directories, quoting, pipelines, redirection, interactive job control, or guest shutdown. [SDK documentation](../SDK.md) and [the ABI constant](../../crates/abi/src/lib.rs) establish ABI 18 and an externally buildable example, with an explicit mismatch failure. Existing documentation and code were inspected; no VM suite was rerun for this research note.

## Primary-source findings

1. **Durability depends on the entire persistence contract.** SQLite's rollback-mode design accounts for reordered writes and torn sectors and places explicit flushes between recovery-critical steps. It depends on flush completion having its advertised persistence meaning. Its crash tests simulate incomplete sectors, damaged data, and reordered writes, then verify an entirely committed or rolled-back transaction. This is a testing pattern to borrow, not a proposal to turn SQLite into a filesystem. [SQLite atomic commit, sections 2, 3, 8, and 9](https://www.sqlite.org/atomiccommit.html).

2. **Recovery itself must survive failure.** SQLite's testing documentation describes advancing crash points across I/O operations and combining failures, including I/O errors or allocation failure while recovering from an earlier crash. A clean reboot after a successful write is a much weaker test. [How SQLite is tested, sections 3.3–3.4](https://www.sqlite.org/testing.html).

3. **Bounded memory and scalable file data can coexist.** littlefs combines small redundant metadata logs with copy-on-write file data; its design explicitly targets power-loss resilience, flash wear, and bounded memory on microcontrollers. This involves space and traversal tradeoffs. Its suitability for flash does not establish that it is the best filesystem for a PC block device. [littlefs design](https://github.com/littlefs-project/littlefs/blob/master/DESIGN.md).

4. **On-disk recovery rules need a precise format specification.** littlefs specifies revision comparison, commit CRCs, alignment, and detection of partially programmed subsequent commits. The specification explicitly calls for sequence comparison to address revision overflow. These details illustrate why a commit flag alone cannot define a reliable recovery protocol. [littlefs on-disk specification](https://github.com/littlefs-project/littlefs/blob/master/SPEC.md).

5. **Typed handles can support a complete application platform.** Zircon checks a handle's existence in the caller's table, object type, and rights. Rights can be reduced on duplication; handles can be transferred. It separates byte streams with partial I/O from bounded message channels carrying handles, and uses jobs for grouped resource and lifetime control. Program loading can live above the kernel. [Zircon kernel concepts](https://fuchsia.dev/fuchsia-src/concepts/kernel/concepts).

6. **Convenient paths need not imply global authority.** Fuchsia gives components tailored namespaces and directory handles. Access starts from objects the component already possesses; object-relative traversal proceeds into children. Client-side interpretation can provide rooted paths and `..` convenience separately from the underlying namespace protocol. [Fuchsia namespaces](https://fuchsia.dev/fuchsia-src/concepts/process/namespaces).

7. **Terminal ownership is separate from command parsing.** FreeBSD describes canonical line input versus noncanonical byte input, bounded queues, foreground jobs, terminal access rules, and delivery of interrupt characters to the foreground job. The useful lesson is to specify who consumes input and restores terminal state; copying every historical signal or ioctl is optional. [FreeBSD termios(4), official source](https://raw.githubusercontent.com/freebsd/freebsd-src/main/share/man/man4/termios.4).

8. **Pipelines need real streams and defined completion semantics.** FreeBSD's shell connects each producer's stdout to the next consumer's stdin; redirection changes input/output references. Its documentation explicitly defines waiting and pipeline status, including `pipefail`. These are observable contracts, not merely parser syntax. [FreeBSD sh(1), redirections and pipelines](https://raw.githubusercontent.com/freebsd/freebsd-src/main/bin/sh/sh.1). POSIX.1-2024's shell language was also located, but full-page retrieval returned HTTP 403; the FreeBSD first-party manual is the directly inspected source used here.

9. **Signed files alone do not establish trustworthy updates.** TUF specifies trusted root metadata, role signatures and thresholds, metadata versions, expiration checks, target hashes/lengths, and root rotation. It addresses rollback, freeze, and inconsistent metadata combinations. A client therefore needs persistent trust state and an explicit time policy, as well as signature verification. [TUF specification, client workflow](https://theupdateframework.github.io/specification/latest/). The official index listed v1.0.36 during research; implementations must pin the selected specification and dependency versions rather than silently track `latest`.

10. **Download, validation, installation, and activation are distinct phases.** RFC 9019 distinguishes these roles and requires resilience to power and network interruption during update; image invocation includes authenticity/integrity verification. It is an IoT firmware architecture, not a PC certification checklist. [IETF RFC 9019, sections 3–4](https://www.rfc-editor.org/rfc/rfc9019.html).

## GenOS recommendations: storage

**First strengthen the existing storage contract and its test harness.** Do this before selecting a larger format. Define separately atomic visibility, acknowledged durability, cancellation, error reporting, and remount behavior. Describe assumptions for sector size, reordering, tearing, flush completion, and device timeout. A QEMU process kill does not by itself simulate loss of the host's page cache or a physical drive's volatile cache.

**Treat a failed final commit flush as potentially ambiguous.** This is an engineering inference to investigate, not a demonstrated current bug: the documented rollback restores old RAM state after any flush error, but the new commit may already have reached the disk before an error or timeout is reported. Restoring RAM is not proof that disk changes disappeared. Add a storage state for unknown commit outcome; block further mutation until safe reconciliation or read-only recovery. Specify which observations are allowed after reboot. Do not promise that every failed mutation is necessarily absent after reboot.

Local evidence for that audit item: [`PersistentFs::commit`](../../kernel/src/storage.rs) writes the committed header and calls `self.cache.flush()?` before updating `active_slot` and `generation` (lines 237–287 at inspection). `BlockCache::flush` writes dirty sectors before calling `ata_flush` (line 175); `ata_flush` sends the flush command then waits for completion (line 762). `PersistentFs::sync` reduces the result to a boolean. [`RuntimeCoordinator::persist_change`](../../kernel/src/runtime.rs) restores the VFS and returns false on sync failure (line 970), while `persistent_write_denied` only consults the existing read-only flag. The [storage contract's Commit and recovery section](../STORAGE.md#commit-and-recovery) describes the RAM rollback. Thus the code path can return failure after the final header was submitted without recording an explicit unknown-outcome state. Reproduction under injected completion/flush failure remains required; the proposed reconciliation and write-blocking behavior is **not implemented** by this research change.

**Then choose the next filesystem against explicit workloads.** Compare an extension of the custom format with reuse of a small existing implementation in an ADR. Evaluate large-file updates, random overwrite, directory operations, space amplification, fragmentation, recovery cost, implementation/FFI burden, and host tooling. Borrow littlefs's bounded metadata and recovery discipline without prematurely mandating littlefs, journaling, or a full B-tree implementation. Keep a read-only decoder and an export/migration path for existing GFS2 volumes.

Proposed first useful-console budgets are at least 1 MiB per file, 1,024 namespace entries, and 255 bytes per complete path on a dedicated larger test volume. These numbers are product targets to validate, not standards requirements; they support small executables, source/text files, and command output while keeping tests tractable. Publish actual chosen limits, memory costs, and large-file behavior before calling the platform useful.

Acceptance work:

- Inject failure before/after every logical write and flush in create, write, truncate, remove, and rename; additionally vary sector tearing, write ordering, and lost writes in a storage emulator.
- After a successful durable return, remount and require exact committed bytes. Before commit, require the prior complete state. At an ambiguous commit boundary, permit only a documented complete old or complete new state; reject mixed metadata/content.
- Inject another crash, I/O failure, or allocation failure during recovery. Preserve forensic images; do not automatically format an unreadable disk.
- Test full storage, file/metadata budget exhaustion, partial reads/writes, nested directories, generation overflow, and corruption of either or both copies.
- Build an independent host parser/checker and compare guest state with it. Test backup/export, restore, and interrupted migration on disposable images.
- Add an atomic replacement operation for editor saves and installation; do not implement a misleading `mv` as unchecked copy-then-delete.

## GenOS recommendations: applications and services

**Use capability-explicit spawn as the native launch model.** Extend the existing typed-handle approach with a versioned launch contract containing executable reference, arguments, optional environment, working-directory authority, stdin/stdout/stderr handles, and resource budget. Validate the whole request before making a child runnable; failure must reclaim every intermediate resource. Continue with static ELF applications while ABI and image-layout contracts settle.

**Implement streams before shell pipelines.** Define bounded byte buffers, partial I/O, readiness/waiting, EOF, broken-peer errors, cancellation, and close semantics. Keep message-oriented IPC for bounded structured requests and authority transfer. A service supervisor can then own process groups, restart budgets, and dependency failure behavior without importing a large component framework.

**Keep authority explicit through path convenience.** `cd`, `pwd`, and relative paths should operate within granted directory roots. The kernel or trusted filesystem layer must enforce confinement; shell canonicalization alone must not be the security boundary. Do not grant every child writable `/USER`, raw devices, or networking by default.

Acceptance work:

- Launch at least three independently built SDK applications from storage, pass arguments and streams, collect exit statuses, and handle invalid ELF/ABI requests without a kernel failure.
- Test nonexistent, stale, wrong-type, cross-process, and insufficient-rights handles; duplication cannot add rights and transfer cannot accidentally duplicate ownership.
- Run repeated child launch/exit/cancel with no monotonic growth in pages, handles, endpoints, or job records; inject allocation failure at every construction stage.
- Test a producer larger than the pipe buffer, slow/early-exiting consumers, peer death, and cancellation while blocked. Every case returns control to the shell without deadlock.
- Bound service restart storms; retain a recovery console when an optional service fails.

## GenOS recommendations: terminal

Deliver this in dependency order:

1. A bounded line editor with backspace/delete, cursor movement, home/end, history, completion, line cancellation, long-input handling, and useful syntax/error messages. Define supported encoding and key sequences. Incomplete or malformed escape sequences must not execute accidental commands.
2. Working directories, quoting/escaping, general named launch, exit statuses, and a small useful utility set: copy, rename/move, text viewer/editor, and storage/process diagnostics.
3. Redirection and pipelines after stream semantics exist; foreground input ownership, cancellation of the whole foreground pipeline, and predictable prompt restoration. Add background jobs and stop/resume only with explicit lifecycle rules.
4. Guest `exit`, `shutdown`, and `reboot` contracts, including session cleanup and storage draining. Keep host QEMU escape instructions distinct from guest controls.

Acceptance should use a visible serial transcript, not only internal proof markers: type/edit/cancel a command; quote an argument; save and reopen text after restart; run a multi-stage pipeline larger than buffers; stop a stuck workload; kill an app that changed terminal mode; continue using the shell. Include pasted multiline input, queue overflow, invalid bytes, and background output while editing a line. Display file names/control bytes safely so arbitrary content cannot masquerade as trusted prompt output.

## GenOS recommendations: trusted distribution

For the local console VM milestone, use developer-built images with recorded toolchain/source identity, image hashes, isolated test disks, and a documented backup/recovery procedure. A repository client, multiuser login, TLS, and general internet exposure need not block this deliberately limited milestone.

Before distributing automatic updates or claiming safe untrusted-network use, select a maintained TUF implementation or rigorously justified compatible design. Keep cryptography and trust verification in reviewed libraries. Define root provisioning, key rotation/revocation, target/ABI compatibility, bounded metadata parsing, trusted-time bootstrap, and persistent anti-rollback state. An unavailable trusted clock must have a documented recovery path; silently ignoring expiration defeats the claimed protection.

Separate verification from installation and boot activation. Stage a new image, durably record activation, retain a known-good recovery path, and confirm boot health before making it permanent. A/B images are a candidate approach, not a mandatory layout. Distinguish authorized rollback to a supported recovery image from an attacker's downgrade to revoked code. Test interrupted download/install/activation, invalid signature/hash, old/mixed/expired metadata, missing time, disk full, failed boot health, and key rotation.

## Scope and deferrals

| Claim | Required scope for these workstreams |
| --- | --- |
| Verified reference kernel | Existing bounded-storage contract is honest and fault-tested; isolation and cleanup are tested; shell remains a recovery/control interface. |
| Useful local console OS | General launch/heap/streams, usable storage and terminal workflow, deliberate limits, save/reboot/recovery, reproducible validation on the named VM configuration. |
| Daily use or untrusted applications/network | Stronger resource/failure isolation, explicit authority, reviewed crypto, identity policy where needed, trustworthy distribution and security maintenance. |
| Supported real hardware | Storage flush/error semantics, device reset and power behavior validated on each named hardware configuration; VM evidence alone is insufficient. |

Defer GUI, full POSIX conformance, `fork`, dynamic linking, Unix signal compatibility, arbitrary filesystems, complete Unicode terminal behavior, fleet-oriented Uptane roles, and automatic internet updates unless a supported workload justifies them. These are scope decisions, not claims that those features are inherently undesirable. GenOS can retain its own architecture while adopting well-tested contracts and failure-testing practices.
