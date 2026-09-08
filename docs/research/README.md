# Roadmap research — September 2026

Research date: **2026-09-08**. Code/evidence baseline: **`5599dc7`**.
These notes inform the [roadmap](../../ROADMAP.md); they do not implement its gates
or report new kernel tests. Facts are linked to primary sources beside the claim;
GenOS recommendations and numerical targets are labeled separately.

- [Kernel ownership, CPU context, firmware and verification](2026-09-kernel-foundations.md)
- [Storage recovery, applications, terminal and trusted distribution](2026-09-console-platform.md)
- [Networking, VirtIO and reference hardware](2026-09-network-hardware.md)

The method was to compare official specifications and first-party engineering
practice with the existing code and retained verification record. Sources include
Intel, UEFI, Rust, seL4, Linux lockdep, Miri, Kani, the Rust Fuzz Book, SQLite,
littlefs, Fuchsia/Zircon, FreeBSD, TUF, IETF, OASIS and QEMU. Search summaries were
not used as substitutes for the cited page text. Retrieval limitations are noted.

## Selection rules

1. Prefer a small, testable contract with explicit ownership and failure behavior.
2. Use the existing Rust kernel as the starting point; require measured evidence
   and an ADR before an architecture rewrite or dependency choice.
3. Borrow established interfaces and testing methods selectively. Full POSIX,
   `fork`, dynamic linking, microkernel conversion, a new filesystem algorithm,
   and new cryptography are not prerequisites by popularity.
4. Distinguish local evidence, remote CI enforcement, long-run qualification and
   physical-hardware evidence. None is interchangeable with the others.
5. Pin the source revision used for implementation. A draft, rolling `latest`
   document or selected reference experiment is not a blanket compliance claim.
6. Treat soak lengths and capacity figures as GenOS acceptance targets to validate,
   not externally certified thresholds that establish absence of bugs.

The old limitations register mixed historical audits with current updates. The
refreshed [current limitations](../KNOWN_LIMITATIONS.md) and roadmap reconcile that
history; the pre-refresh audit remains available under `docs/history/`.
