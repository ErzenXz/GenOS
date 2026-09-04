# ADR 0002: Normalize x86 exception entry before broadening the kernel

Status: proposed for review. Architecture proposal: issue #5. This change is
stacked on the unmerged quality-first roadmap in PR #4.

## Problem

The GenOS 0.49 IDT initially installed a bare `iretq` for most vectors. Only
#DF, #GP and #PF had explicit fault entries. A CPU-pushed error word changes
the return-frame layout. Unhandled exceptions must not guess that layout or
retry the faulting instruction indefinitely.

## Decision and ownership

Keep the current custom assembly boundary, but install one explicit stub for
each of the 256 vectors. `kernel::exception` owns the pure frame, classification,
IST and PIC-acknowledgement contracts. `interrupt_entry.S` constructs the frame.
`interrupts.rs` owns dispatch. `arch.rs` owns BSP-only IDT/TSS construction.

The shared frame contains 15 saved general-purpose registers, vector, error,
RIP, CS, RFLAGS, RSP and SS. Its size is 176 bytes. Offsets are tested.
CPU-error vectors are 8, 10, 11, 12, 13, 14, 17, 21, 29 and 30. Other entries
push a synthetic zero. A Rust constant supplies the assembly mask. Software
`int` is not a valid way to test a CPU-error-code exception.

Entry clears DF, preserves the GPRs, samples CR2 before calling Rust and aligns
the SysV call stack. The ordinary IRQ0 path deliberately retains `UserContext`
without the extra vector/error words. The syscall ABI does not change.

Recoverable Ring 3 exceptions pass through the existing exact-current-process
termination path. That path records the vector and exit status and returns
through the existing nonlocal userspace trampoline. NMI, double fault, machine
check, reserved vectors and unknown external-controller vectors are fatal.
Ring 0 exceptions print the normalized CPU frame, clear IF and halt.

Separate 16 KiB IST buffers serve double fault, NMI, machine check and debug.
The ordinary interrupt stack remains 64 KiB. These buffers are static, disjoint
and BSP-owned. They add 64 KiB of bounded storage. They are not guard-paged.

Unowned real PIC interrupts are masked before acknowledgement. Spurious IRQ7
sends no EOI. Spurious IRQ15 acknowledges the master cascade only. This policy
is specific to the remapped 8259 PIC; it does not substitute for APIC/MSI-X work.

## Safety, synchronization and failure

The IDT is initialized with maskable interrupts disabled and remains privileged.
No new resource authority, allocation, DMA or userspace pointer copy is added.
Assembly owns the saved frame until return or nonlocal process exit. Rust borrows
it only during dispatch. The existing single-core, no-red-zone kernel target
and GPR-only context contract remain prerequisites.

Unsafe code touched: IDT function-table reads, IDT/TSS initialization and stack
addresses, entry/return assembly, and PIC command/mask I/O. Other inherited
unsafe operations in `arch.rs` and the normal IRQ adapters are unchanged.

This does not resolve all nested-entry or concurrency risks. Re-entry into the
same IST, exception-stack guard pages, XSTATE ownership and SMP/per-CPU entry
require separate work. A machine-level event must never masquerade as successful
process-local recovery.

## Verification contract

Run:

```sh
cargo test -p kernel --test exception
python3 -m unittest discover -s tools -p test_exception_harness.py -v
python3 tools/test_exception_entry.py --mode user --fault de
python3 tools/test_exception_entry.py --mode user --fault ud
python3 tools/test_exception_entry.py --mode user --fault gp
python3 tools/test_exception_entry.py --mode user --fault pf
python3 tools/test_exception_entry.py --mode kernel --fault de
python3 tools/test_exception_entry.py --mode kernel --fault ud
python3 tools/test_exception_entry.py --mode kernel --fault gp
python3 tools/test_exception_entry.py --mode kernel --fault pf
make test
```

The CPU matrix makes a disposable source copy. It changes only the existing
deliberate fault instruction and its exact expected result, or injects one
kernel fault after interrupt initialization. It never modifies the production
exception handler, process termination, scheduler or cleanup implementation.
Every replacement requires exactly one matching source anchor.

User probes retain the existing two healthy peers, private-mapping checks,
preemption checks, exact fault status and frame reclamation. Kernel probes
require one complete serial frame, an explicit halt, no continued boot and a
live, non-reset QEMU process. Each case retains the source fixture diff, image
hash, commit, tool versions, build log, serial log and QEMU command. Evidence
parser tests reject missing markers, wrong privilege, wrong vectors, duplicate
faults and resumed kernel execution.

Passing these tests proves only these vectors on this reference VM. It does not
prove physical machine-check/NMI recovery, every architectural exception, or
all F1 acceptance criteria. Full existing CI remains required independently.

## Roadmap status and remaining boundary

This advances F1's normalized entry, deliberate fault handling, stack separation,
and deterministic #DE/#UD/#GP/#PF evidence. F1 remains open. In particular:

- the IDT is page-aligned but **not CPU-enforced read-only**;
- dedicated emergency-stack selection is tested, not every hardware fault path;
- broader F2 CPU permissions and F5 synchronization remain open;
- no networking, storage, application, physical-hardware or GUI gate is closed.

No roadmap item is considered verified merely because this document or a new
workflow exists. The exact commit's successful CI results are the evidence.

## Alternatives, migration and rollback

Adding only #DE and #UD would leave other error-code vectors unsafe. Switching
to a compiler interrupt ABI would also require redesigning the scheduler's
current nonlocal return. The shared assembly contract is the smaller change.

There is no userspace ABI or storage-format migration. Merge the base policy PR
first, then retarget this implementation to `main`. Neither branch is merged
automatically. Reverting the entry commit restores the experimental baseline,
not an equally secure configuration.

The series separates pure contracts, entry wiring, and CPU verification. The
whole PR exceeds 500 lines because the architectural frame and its auditable
positive/negative system evidence must be reviewed together. No performance
improvement is claimed.
