# ADR-0013: Bounded user CPU context

- **Status:** Proposed
- **Date:** 2026-09-28
- **Decision owners:** architecture/process maintainers
- **Related issue:** next-roadmap-item continuation in the local development chat
- **Related roadmap gate:** F1.2; F1.4 remains separate
- **Supersedes:** None
- **Superseded by:** None

## Context and decision

The eager legacy FXSAVE64 image omitted application-writable visible segment
selectors. A peer could leave DS/ES/FS/GS values for the next process. Firmware
FS/GS bases, LDT/debug state and fast syscall entry settings were also retained
without an application contract.

Preserve DS/ES/FS/GS in an eight-byte process-owned image beside the existing
512-byte FXSAVE64 image. Capture before Rust on every Ring 3 entry, install
kernel selectors, and restore before user return. The admitted single-BSP
profile initializes each process with DS/ES `0x2b` and null FS/GS. All admitted
descriptors have zero bases; LDTR is null, FSGSBASE remains disabled and no API
writes bases. Zero/read back FS_BASE, GS_BASE and KERNEL_GS_BASE at boot.

Disable/read back debug control and addresses at boot. No debugger API is
provided; user MOV DR is privileged and user #DB terminates that process.
Disable EFER.SCE and zero/read back SYSENTER MSRs where SEP exists. The owned
application entry remains `int 0x80`. A readback mismatch halts before admission;
an unexpected kernel fault deliberately halts through the installed IDT.

Pending unmasked x87 exceptions remain in the private FXSAVE image across
non-waiting save/restore and scheduling. The first waiting user instruction
raises #MF and terminates the process. Immediate user SIMD #XM also terminates
the process. Kernel #MF/#XM are fatal. This adds no signal/restart API.

## Invariants, ownership and failure

The process cannot move while its assembly-published state pointers are live.
IF is clear on the admitted BSP outside Ring 3; all GPRs are saved before helpers
clobber RAX. Selector MOVs and FXSAVE/FXRSTOR preserve flags. Helpers touch only
the current process's supervisor-mapped private images. Kernel-origin IRQs do
not capture over those images. The return trampoline clears both pointers.

The kernel uses soft-float and no FS/GS-relative TLS. All user-loadable GDT
descriptors remain flat; no LDT or nonzero-base API can be added independently
of this contract. Exit/fault/kill teardown reclaims the existing address space;
state images need no allocation, pins or separate cleanup. Construction/reuse
always initializes both images. This leaves NMI/same-IST/return-fault behavior
and raw-global ownership under F1.4/F4/F5.

## Alternatives and costs

Leaving selectors shared has no coherent process-state contract. Resetting them
at every scheduling boundary would break legitimate application selector loads.
General TLS/debugger virtualization would require additional base ownership,
SWAPGS/NMI ordering, authority and tests before current applications need it.
The chosen fallback preserves visible selectors while keeping bases fixed zero.

The selector image adds eight payload bytes per process, with existing structure
alignment padding, and fixed selector MOVs on entry/return. It adds no allocation
or variable IRQ work. No latency improvement is claimed. Broader XSAVE, CET,
UINTR, SMP, physical hardware and speculative-execution claims remain excluded.

## Compatibility and rollback

The public ABI, `UserContext`, ELF and storage formats remain unchanged. Existing
ABI-18 programs receive deterministic initial selectors. A future TLS or debugger
interface needs an explicit ABI/policy extension. Roll back a matched older
kernel/userspace bundle; it reintroduces the old selector/firmware-state limits,
so its evidence cannot retain this qualification claim.

## Verification

Host tests assert selector layout/initial state and reject every corrupted byte.
The six-process production-loader fixture observes initial/private selectors,
FS/GS-relative reads, direct/yield syscalls, PIT preemption, pending #MF, fresh
reuse and exact frame reclamation. Separate exception fixtures prove user
#MF/#XM cleanup and kernel fatal behavior. CPU fixtures test forbidden fast
entries and real readback/reset of poisoned bases and enabled debug control.
Evidence parsers reject absent, duplicate, forged and reordered records.

Exact campaign commits, results and retained failures are recorded in
[VERIFICATION.md](../VERIFICATION.md). Full F1.2/physical qualification and F1.4
are not established merely by configuring these tests. The workflow matrix adds
the FP cases but still needs authorized publication and remote execution.
The pinned TCG user/kernel #XM probes return without trapping and are retained
as failures. MXCSR readback and the diagnostic ELF establish the emulator limit;
no vector-19 containment proof or release promotion is claimed.

## References

- [CPU inventory, assembly boundaries and validation](../CPU_STATE.md)
- [CPU feature variants](../CPU_FEATURES.md)
- [Intel architecture manuals](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html)
- [AMD system-programming manual](https://www.amd.com/content/dam/amd/en/documents/processor-tech-docs/programmer-references/24593.pdf)
