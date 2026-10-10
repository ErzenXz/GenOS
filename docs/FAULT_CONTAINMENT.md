# Exception nesting and return containment

This is the bounded single-BSP contract for roadmap F1.4. First kernel-origin
faults, NMI and machine checks halt. Supported user-origin faults terminate the
current process and reclaim its resources. Nested entries and double faults
halt in assembly. There is no signal, restart, NMI service or machine-check
recovery interface. Exact qualification and retained failures are recorded in
[VERIFICATION.md](VERIFICATION.md).

## Entry and stack ownership

Every IRQ, generic exception and `int 0x80` stub saves all fifteen GPRs and
clears DF/AC before claiming entry ownership. IF is clear. IRQ/syscall frames
retain the twenty-word `UserContext`; exceptions add vector and error words
before the architectural return frame. CPU-supplied error codes are retained;
other vectors receive a synthetic zero. The guard uses Rust-generated offsets.

The first entry publishes its vector and marks the BSP entry active. It then
redirects all five live TSS IST slots to the existing guarded 16 KiB
double-fault stack before saving user CPU state or calling Rust. Original tops
remain in permanent assembly-owned configuration. The first handler therefore
runs on its original IRQ, privilege, NMI, machine-check or debug stack; any
subsequent hardware IST entry uses the reserved terminal stack.

A nested entry observes the active claim and never calls Rust, saves/restores
FP state, allocates, acquires a lock or accesses process/subsystem state. A
first #DF also goes directly to this terminal path. No typed Rust frame or
Rust handler ever lives on the double-fault stack. Reusing its top for later
terminal hardware frames cannot overwrite a live Rust borrow there. Seven
stacks and fourteen [guard pages](KERNEL_STACKS.md) remain reserved.

The short assembly claim/release windows can be interrupted by NMI before
every IST slot has changed. Such an entry still halts before Rust; its frame
can use an original emergency top. The qualified handler windows begin after
the redirect completes. This does not promise that arbitrary CPU failure in
any assembly instruction can produce a complete diagnostic.

Normal direct returns restore user CPU state, restore all original IST tops
and clear the active flag last, then restore GPRs and execute IRETQ. Nonlocal
process exits release the claim before the scheduler trampoline. The Rust
handler's frame borrow has ended before release. A first fault at IRETQ is
therefore a new kernel-origin fatal entry. A fault from IRETQ attempted inside
an active handler is nested and takes the assembly terminal path. Intel defines
IRET as unblocking NMI even when it faults; the dedicated fixture injects a
second NMI after the failed return to test this boundary. See the
[Intel system-programming manual](https://cdrdv2-public.intel.com/825750/326019-sdm-vol-3c.pdf).

## Terminal reporting and machine checks

The terminal path prints vector, parent vector, error, RIP, CS, interrupted
RSP, CR2 for #PF and the saved-frame address, followed by
`EXCEPTION_TERMINAL_HALT`. UART polling is bounded to 65,536 attempts per byte;
an unavailable transmitter halts immediately after that bound. Linked fixed
strings, sixteen-digit hexadecimal output and at most three call levels bound
the work. A CLI/HLT loop cannot resume the interrupted handler. Diagnostics may
be partial if serial hardware or the terminal stack fails.

Boot enables and reads back CR4.MCE only when CPUID reports both MCE and MCA,
emitting `CPU_MACHINE_CHECK_READY available=1 policy=halt`. Otherwise it leaves
MCE disabled and reports `available=0 policy=unsupported`. The reference proof
requires the former. The kernel does not inspect/clear banks or acknowledge
MCIP for recovery. An injected first #MC is fatal; a synchronous fault inside
that handler is nested. Physical corruption, CPU shutdown conditions, a second
machine check with MCIP active, arbitrary event storms and SMP are outside this
contract.

## Native user return state

Before publishing a process for a slice, and when accepting a syscall/timer
frame, the kernel requires exact native CS/SS, an executable RIP, an RSP inside
the owned user stack, and the admitted RFLAGS mask `0x00250fd7`. Bit 1 and IF
must be set. IOPL, NT, VM, VIF/VIP and reserved bits are rejected. Arithmetic
flags, TF, DF, RF, AC and ID remain admitted; this is not exhaustive testing of
their interactions. Captured user frames additionally require the process CR3.

An invalid internal context reports `USER_RETURN_REJECTED origin=kernel
policy=halt` before entering Ring 3. Invalid captured user state reports
`USER_RETURN_REJECTED origin=user policy=terminate`, records a process-local
#GP identity and uses the existing termination/reclamation path. These are
software denials, not fabricated CPU exception frames. Hardware fixtures
deliberately corrupt return words after validation to require real #GP at
the actual IRETQ site. Public ABI and frame layouts are unchanged.

## Verification

```sh
cargo test -p kernel --lib exception
python3 -m unittest discover -s tools -p test_fault_containment_harness.py
python3 tools/test_fault_containment.py
```

The immutable-source harness has twenty cases: scoped syscall borrow denial, nested IRQ #PF, syscall #GP,
same-IST #PF, IRQ NMI/#MC, #MC-handler #GP, failed NMI IRET plus second NMI;
four corrupt initial/syscall/IRQ returns; a successful IRET control with NMI re-entry; real #DF; initial flag/RSP/SS denial; user NT
denial with cleanup; and first NMI/#MC. External events use QEMU's documented
[QMP NMI](https://www.qemu.org/docs/master/interop/qemu-qmp-ref.html#command-inject-nmi)
and [monitor machine-check injection](https://www.qemu.org/docs/master/system/monitor.html).

The guard, classification, return validator and cleanup remain production
code. The fixtures alter only deliberate fault sites, barriers, return words
and the #GP gate used to provoke a real #DF. Evidence checks exact fault/parent
identity, stack geometry, active-window ordering, return instruction address,
explicit halt and a still-running guest after containment. Reset, exit, paused
guest, missing/duplicate/forged records or resumed dispatch fail qualification.
The pinned TCG implementation leaves NMI blocked when IRET faults and admits
a noncanonical return RIP before its later fault. Those two strict cases remain
qualification failures. A successful IRET control tests second-NMI entry without
substituting for the failed-IRET requirement. Initial/syscall/IRQ corrupt CS
cases independently test real return faults. The target RSP is validated in
software: loading it during IRET does not itself require a #SS. Earlier probes
that assumed otherwise are retained as fixture failures, rather than emulator
qualification gaps. This follows the IRET operation and exceptions in the
[Intel instruction manual](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2a-manual.pdf).
Detailed results and source analysis are in [VERIFICATION.md](VERIFICATION.md).

Build logs, fixture diffs, hashes, commands, QMP transcripts and success/failure
manifests remain under `build/fault-containment-evidence`. Configured CI jobs do
not establish remote execution. See [Proposed ADR-0014](adr/0014-terminal-entry-ownership.md).
