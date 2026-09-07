# GenOS 0.56 integration verification — 2026-09-08

PR #4 (quality gates), PR #6 (normalized exception entry), and PR #7 (GenOS 0.56 integration) are merged to main. This integration preserves the later GenOS 0.50–0.55 network/SDK work and adds CPU page protections and transactional memory construction. It advances the roadmap without claiming a finished or hardened operating system.

## Proven locally

- Pinned Rust 1.97.0 on Apple Silicon macOS, native QEMU 11.1.1 and EDK2.
- 140 Rust host tests and eight Python evidence-parser regressions pass.
- Formatting, documentation links, strict Clippy for every shipped target, and all CI release-target checks pass. The earlier kernel-binary lint failures are resolved.
- All eight original CPU exception cases pass: user/kernel divide error, invalid opcode, general protection and page fault.
- Six additional CPU protection cases pass with exact fault-address checks: user data/stack NX; kernel IDT/text write protection; SMEP execution; SMAP data access. The default CPU reports optional features absent, while these six probes use `-cpu max` and require both enabled.
- Native QEMU storage creation/restore, corruption/repair/read-only recovery, real serial input, concurrent TCP/loss/reordering, IPv6 SLAAC/DAD/echo and external SDK execution pass.
- Allocation injection fails all ten production process-construction allocation points and restores the live-frame baseline. Host tests cover every allocation in a branched supervisor clone, 1,025 reclaimed frames, and 20,000 randomized fragmented allocation/free operations.
- The normal user disk is separate from all corruption, benchmark and rollback fixtures. Validation packaging/boot errors also attempt production-image restoration and report restoration failure explicitly.

## Reproduction and evidence

`make test` runs the host and native system suite, including memory rollback and the six new protection cases. `cargo xtask test-protections` runs only those six cases. The existing exception workflow separately runs the original eight cases. The workflow definitions remain unchanged; expanded protection testing uses the existing build-and-boot entry point and artifact include patterns.

The CPU harness requires committed source and retains unique per-run manifests with commit, compiler/QEMU versions, fixture patch, image hash, command and serial output. Local evidence is under `build/exception-evidence/`; CI also retains `build/serial-protection-*.log` and `build/serial-protection-manifests.log`. System logs include `serial.log`, `serial-network.log`, `serial-sdk.log` and `serial-memory.log`.

The [development benchmark](PERFORMANCE.md) records the earlier GenOS 0.55 baseline (3.886-second median including firmware and validation work). It is not a current-release or comparative-performance claim. Re-measure with `make bench` on the current tree.

## Remaining limits

F0's separate release-image boot still remains open. CPU protection coverage does not establish physical-frame-wide W^X, missing-NX or mixed optional-feature VM proofs, emergency-stack guards/nesting, XSTATE, or physical hardware support. The allocator manages up to 8 GiB of usable frames across 64 regions; firmware-map truncation, larger metadata, per-owner frame tokens, sensitive-page scrubbing, contiguous allocations and SMP remain future work. Validation probes still run in normal development boot.

Production TCP/IPv6 sockets and DNS, identity and reviewed TLS, signed packages, general application launch, userspace graphics/accessibility, ACPI/SMP, xHCI/NVMe, suspend/resume and real-machine validation remain roadmap gates. Run the current console OS with `make run`.

The first integration CI run exposed stranded coalesced VirtIO RX completions. A production-driver queue test reproduced it: one IRQ with two used descriptors delivered only the first. RX now drains the announced bounded batch before waiting for another IRQ, preserves correct recovery accounting, and rejects impossible used-index advances. The existing regression budgets are unchanged.

After RX batching removed artificial delays, the native test exposed a second timing issue: the headless bootstrap used loop iterations as timer ticks, allowing a real network wait to expire before a host packet arrived. The bootstrap now uses the hardware/fallback timer while independently bounding work and elapsed time. The same network gate then observed real readiness wakeups and passed without relaxed budgets.

An intermittent Linux CI persistent-write failure did not recur in eight isolated native serial boots or the next instrumented CI run. ATA status tests did expose two concrete protocol errors: BUSY status could be treated as a completed error, and non-busy DRQ status could be treated as command completion. Those cases now wait correctly. Bounded polling and explicit command-error/timeout diagnostics remain; the exact cause of the earlier CI write failure was not established.

GitHub validation for PR #7: [CI](https://github.com/ErzenXz/GenOS/actions/runs/34171042822) and [exception proofs](https://github.com/ErzenXz/GenOS/actions/runs/34171042829) passed on the final reviewed branch head. The prior local main/history are retained in backup branches.
