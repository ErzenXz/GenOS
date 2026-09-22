# GenOS current limitations

**Updated 2026-09-22; exact foundation source and evidence are recorded in [VERIFICATION.md](VERIFICATION.md). Level: Experimental.**
This is the current register, not a cumulative historical audit. The
[previous register](history/2026-09-08-limitations-before-refresh.md) preserves older
wording that described already-replaced implementations. Research and planned fixes
below do not mean those fixes have been implemented.

The [roadmap](../ROADMAP.md) owns dependencies and acceptance criteria. The
[verification record](VERIFICATION.md) owns reported results and exact commits.
Neither test counts nor an implemented mechanism establish a stable or hardened OS.

## Release and supported configuration

The demonstrated environment is a single active kernel CPU in the x86_64
QEMU/UEFI reference setup. Normal debug/release images, selected CPU feature/fault
cases and one/four-vCPU BSP admission have local evidence. There is no supported
physical-machine matrix, complete untrusted-workload threat model, stable ABI/storage
support period or qualified daily-use release.

A reliable local-console milestone is the next product target. General Internet
operation, secrets, hostile applications/devices and automatic updates need additional
security qualification. Do not infer them from local boot or plaintext test traffic.

## Evidence and integration

The implementation record reports host tests, Python evidence checks, parser CLI
and mutation runs, and scoped storage/network/memory/CPU/BSP suites. Exact source,
totals, failures and successful reruns belong in [VERIFICATION.md](VERIFICATION.md).
Those are scoped local results. The VM candidate now has pinned arguments and an
exact tool/firmware preflight; independent reproduction remains missing.
New independent CI jobs and a weekly long-validation workflow are configured locally,
but the branch push was rejected because the OAuth credential lacks `workflow` scope.
No draft PR was created and these new jobs have not supplied remote evidence.

Coverage-guided fuzzing, suitable Miri/model-checking coverage, complete operation and
fault matrices, 1000 boots, sustained qualification, independent reproduction and
required-check enforcement remain open. The configured 180-minute long-run job is
not a completed run and cannot establish the proposed 24/72-hour soak durations.
**Roadmap:** F0, F6, F7, C5, Stage 10.

## CPU state, boot and memory protection

Normalized exception entry, deliberate user fault termination/kernel halt, emergency
stacks, an IDT protected by the CPU, NX/WP, optional SMEP/SMAP and a kernel-owned boot
stack are implemented. The former bare catch-all entry and firmware-stack overflow
are not the current design.

BootInfo/map admission now rejects oversized/truncated, overflowing, overlapping
maps and unretained kernel/boot/initrd ranges; unknown/runtime memory stays reserved.
It still trusts an accessible typed handoff. The [kernel ELF loader](KERNEL_ELF.md)
now validates a bounded immutable load plan before allocation/copy. Exhaustive
firmware map-growth/stale-key retry failures remain untested.

The reference now eagerly preserves x87/MMX/SSE state in a private 512-byte
process image and initializes clean state on reuse. OSXSAVE/PKE are disabled;
kernel Rust remains soft-float. The [CPU contract](CPU_STATE.md) records real
preemption/syscall/fault/reuse evidence and its limits. It does not establish
FS/GS/debug register virtualization, unmasked FP exception behavior or support
for arbitrary physical CPUs/ISA combinations.

Seven stacks now have fourteen inaccessible page guards; see [KERNEL_STACKS.md](KERNEL_STACKS.md).
Remaining gaps include stack high-water accounting, complete CPU
state and feature qualification, same-IST/NMI/fatal nesting and return-fault behavior,
and a broader unsupported/mixed-feature matrix. Current explicit mappings reject W+X, but
physical aliases still prevent a physical-frame-wide permission claim.
**Roadmap:** F1, F2.

## Memory ownership and concurrency

The bitmap allocator represents up to 8 GiB of managed usable frames in 64 ranges.
Current paths zero before granting, scrub before reuse, reject selected invalid or
duplicate releases and roll back tested construction failures. Allocator accesses
use scoped local IRQ masking. `mem` reports real managed-frame counters.

The [grant ledger](FRAME_GRANTS.md) now checks owner/allocation identity, rejects
stale grants and duplicate user aliases, pins mapped user frames, and validates
inactive-root teardown before unlink/scrub/reuse. It bounds live dynamic allocations
to 8192 pages (32 MiB including tables), separately from the bitmap's 8-GiB address
coverage. The ledger uses at most 328,000 bytes and bounded linear scans; worst-case
IRQ latency and memory-pressure behavior are not qualified. Arbitrarily corrupted
page-table topology is outside the safe construction contract.

Sharing, device/DMA pins, kernel direct-map alias retirement, full early/runtime
policy, contiguous allocation and per-process quotas remain incomplete. Physical
release remains unsafe because callers must retire raw references and device uses.
`/MEMORY.STATUS` now provides an immutable snapshot for each read-only open handle;
partial reads and metadata retain that captured version until close/revocation.

Other process, address-space, scheduler, runtime and device state still needs a full
ownership/context audit. Local IRQ masking does not protect NMIs or other CPUs.
Per-CPU state, lock-order enforcement and acknowledged cross-CPU TLB retirement are
not implemented; SMP remains disabled. **Roadmap:** F3, F5, H-SMP.

## Module and unsafe-code review

Endpoint authority and pathname rules now have production modules with executable
host tests. Much process/context/loader/syscall/lifecycle coordination remains in
`kernel/src/userspace.rs`, and other shared globals and presentation dependencies
need decomposition. The generated [lexical inventory](unsafe-inventory.json)
records the current unsafe/assembly sites and their source context. It is not a
caller-invariant audit, Rust soundness proof or measure of relative OS safety.
**Roadmap:** F4.

## Application and terminal platform

ABI 18, Ring 3, typed handles, exact request identities and cleanup are real mechanisms.
The SDK can build an external small ELF and execute it through a test image. The shell
is readable in normal mode, has bounded help and serial clearing, and supports the
commands in [TERMINAL.md](TERMINAL.md).

The general application/console platform is still missing: named launch from storage,
arguments/environment and tailored directory/stdio authority, userspace heap/mapping
growth, composable streams, service discovery/supervision, stable SDK compatibility,
working directories, complete line/escape editing, quoting, redirection, pipelines,
foreground cancellation, background job rules, scripting and guest shutdown/recovery.

There are four managed asynchronous process slots including the shell. Each process
has 20 unified handle slots, with four file and four socket handles. There is one
published endpoint per process and small fixed message/queue budgets. The current
scheduler does not establish fairness/priority behavior for broad workloads, quotas,
priority inversion or multicore scaling. Full POSIX, fork and dynamic linking are
possible later design choices, not requirements to copy another OS. **Roadmap:** C1–C5.

The linked validation shell uses 32,656 of its 32,768 executable bytes (112 bytes
spare); the normal shell uses 17,200 bytes. Further validation growth needs shared
or separate test programs within explicit image budgets. Clippy alone does not
establish that an executable fits; actual linking remains required.

## Storage integrity and scale

Current persistence uses two bounded GFS2 snapshots on the ATA/MBR reference path,
with inspection, repair and read-only recovery. The VFS allows 32 nodes, 64-byte paths
and 512-byte files. It has no general file-data allocator, scalable metadata model,
atomic replacement/rename contract, production backup/migration policy or complete
fault matrix at every mutation/recovery boundary.

The uncertain final-commit outcome is now reproduced and contained: a device
failure quarantines the volume, discards cached writes, restores the last
acknowledged RAM view and requires remount before further writes. Remount may find
the old or new complete generation after an unacknowledged write. There is no live
reconciliation. Snapshot application is transactional even on a late invalid entry
or full VFS. See the [storage contract](STORAGE.md).

The test disk model must also distinguish QEMU termination from physical loss of
volatile caches. The host tests cover errors before/after all 44 commit operations
in two cache models and selected torn writes/second failures. Arbitrary reordered
or lost sectors, physical ATA flush/error behavior, format growth, backup/export
and interrupted migration still require qualification.
**Roadmap:** S1, S2, C4, C5.

## Networking and devices

Modern VirtIO/MSI-X, IPv4/DHCP/ICMP/DNS A, bounded TCP clients/concurrent passive
streams, socket waits, and selected loss/reordering/congestion tests exist. Four
passive slots are global, with a two-slot per-owner limit; socket send/receive queues
are 128 bytes. The control-plane IPv6 work includes SLAAC/DAD/router echo. Claims
that there is no MSI-X or no IPv6 at all describe an older baseline.

Missing work includes general longer-lived streams and larger flight/windows,
broader retransmission/persist/reset behavior, fairness across process owners,
readiness sets, complete device reset/quiescence and fault handling, and reproducible
performance budgets. IPv6 application sockets, AAAA, IPv6-only initialization,
address selection, neighbor/route renewal and PMTU/ICMP error policy remain incomplete.
No TLS, production Internet profile or secure-network claim exists. **Roadmap:**
5.4E, 5.5, SEC1–SEC3, H-DMA.

## Hardware, trust and later products

There is no qualified physical NVMe/xHCI/APIC platform, full ACPI power policy,
SMP, IOMMU containment, general hotplug, suspend/resume, battery/thermal support,
Wi-Fi or audio platform. Existing legacy devices remain explicit development/recovery
paths; one VirtIO NIC does not make the whole VM modern-only.

User/service identity, filesystem ownership/permissions beyond the current coarse
policy, attenuated delegation, CSPRNG, reviewed trust/time/TLS, signed package/update
activation, key rotation and security maintenance are future gates. A compromised
process must not gain unrelated resources merely through a global pathname or PID.

A compositor, GPU platform, graphical applications and accessibility/UI qualification
remain deferred. They do not gate a console-only product, but graphics may not bypass
the application, authority, storage and recovery contracts. **Roadmap:** Stages 6–10.

## Performance and updating this register

There are scoped scheduler/context and validation-boot measurements, not a complete
normal-console latency, memory, throughput or hardware comparison. Record exact
workloads, raw samples, spread and feature differences before making comparative claims.

Update this register with each changed public contract. Remove a gap only with its
implementation and scoped evidence; distinguish confirmed failures from audit inferences,
configured tests from executed tests, and local results from integrated releases.
