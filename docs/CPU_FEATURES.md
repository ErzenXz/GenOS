# CPU admission and instruction contract

The reference CPU is the explicitly versioned `qemu64-v1,smep=on,smap=on` under
the machine, firmware, memory, core count and TCG settings in
[`reference-vm.conf`](../tools/reference-vm.conf). Its feature names do not imply
support for arbitrary x86 hardware or all instructions that CPUID advertises.
Applications use the GenOS ABI and the admitted legacy register-state subset.

## Required and optional features

The firmware-to-kernel ABI requires x86_64 long mode, an operational firmware
stack and the SysV64 boot handoff. Before shared state is initialized, the
[BSP admission gate](SINGLE_CORE.md) checks that leaf 1 exists and advertises MSR
and APIC support, then verifies the BSP bit of IA32_APIC_BASE. Unsupported or
non-BSP entrants halt with IF clear, without touching the shared UART or tables.
Those prerequisite combinations have host policy tests; the BSP VM harness
separately tests normal entry, duplicate entry and an injected non-BSP sample.

After admission, x87, MMX, FXSR, SSE and SSE2 are all mandatory. Missing any
produces `CPU_XSTATE_UNSUPPORTED required=x87,mmx,fxsr,sse,sse2` and halts before
GDT/IDT publication or process creation. NX is mandatory before installing kernel
and user permission policy. Missing NX produces
`CPU_PROTECTIONS_UNSUPPORTED required=nx` and stops boot. CR0.WP is mandatory
and verified, rather than a separately advertised CPUID feature.

SMEP and SMAP are optional, independently detected and enabled when present.
Their absence is reported explicitly in `CPU_PROTECTIONS_READY`; ordinary
supervisor/user PTE separation, NX and write protection remain required. A
mixed-feature boot is separate evidence and never qualifies as the stronger
reference configuration. Normal, SMEP-only, SMAP-only and neither-feature cases
must reach the operational normal shell with their exact reported bits.

## Application instruction boundary

The [eager state policy](CPU_STATE.md) enables x87, MMX and SSE/SSE2. It preserves
their state through actual preemption, direct/yield syscalls, process fault and
reused process storage. Kernel Rust uses the soft-float target; handwritten SIMD
is confined to the save/restore boundary and Ring 3 validation program.

The following CPU modes remain disabled even when a larger CPU advertises them:

| State or instruction | Kernel control | Required Ring 3 outcome |
| --- | --- | --- |
| AVX and other XSAVE-dependent state | CR4.OSXSAVE = 0 | AVX instruction raises #UD |
| XSAVE and XGETBV | CR4.OSXSAVE = 0 | Each instruction raises #UD |
| User protection-key writes | CR4.PKE = 0 | WRPKRU raises #UD |
| Direct FS/GS base writes | CR4.FSGSBASE = 0 | WRFSBASE and WRGSBASE raise #UD |
| Debug-register access | CPL3 cannot execute MOV DR | #GP with normalized zero error |
| Model-specific registers | CPL3 cannot execute RDMSR | #GP with normalized zero error |

These rules follow the control-register and instruction definitions in the
[Intel architecture manuals](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html).
Clearing these enables prevents untracked state from becoming available merely
because firmware used a more capable CPU. It does not establish FS/GS selector
virtualization, TLS APIs, debug-register virtualization or a complete register
inventory. Those remain separate F1.2 work. No kernel syscall provides arbitrary
MSR/debug/control-register writes.

User RFLAGS.AC is not trusted: CPL3 can set it with POPFQ. Every IRQ, exception
and syscall entry saves the user frame, clears DF and clears the live AC bit
with a bounded PUSHFQ/AND/POPFQ sequence before calling Rust or saving XSTATE.
This sequence also works on a CPU without CLAC/SMAP. It changes the kernel's live
flags, not the saved user frame; IRET may restore the application's AC value,
and the next entry clears it again. SMAP copy policy must never depend on the
incoming user value. This statement is an entry-source audit; a hostile AC/DF
transition campaign is separate evidence and is not implied by the ISA cases.

## Reproducible qualification

```sh
cargo test -p kernel --lib xstate
python3 -m unittest discover -s tools -p test_cpu_feature_harness.py
python3 tools/test_cpu_features.py
```

The CPU harness requires a clean commit and retains an independent source
archive, deliberate fixture diff, exact QEMU arguments, image hash, serial log
and success/failure manifest for each case under `build/cpu-feature-evidence`.
The matrix contains eighteen cases: six missing-feature CPUs, all four SMEP/SMAP
combinations, and eight isolated unsupported/privileged instructions. Hardware
variants change the CPU argument explicitly; they do not replace the reference
CPU or silently count as reference qualification.

The missing-feature cases turn off one actual QEMU CPU feature and retain the
production boot and admission code. Only an exact kernel rejection counts as a
pass. A firmware fault, reset, timeout, missing marker or continued application
admission fails; some firmware may itself require these architectural features.
Such failures must be retained and described, not relabeled as successful kernel
admission tests.

ISA cases use QEMU's `max` CPU as an explicitly named test variant. Each extension
case checks its real CPUID feature bit before launching the faulting application;
MOV DR instead uses the architectural privilege rule. The fixture replaces only
the existing deliberate Ring 3 fault instruction and its expected result. It
keeps production entry, fault classification, process cleanup, process isolation
and frame reclamation. Exactly one expected #UD/#GP, the faulting process's
identity and all subsequent isolation/reclamation markers must match. Separate
XSTATE stress faults are suppressed only in this disposable isolated fixture.

Host tests reject absent, forged, duplicate and reordered evidence, mismatched
feature bits, missing hardware proof, wrong fault privilege/vector and missing
cleanup. Configuring a campaign is not evidence that it passed: retain and cite
the exact campaign commit before closing the roadmap criterion. None of these
emulated cases qualifies physical hardware, every CPU model, arbitrary firmware,
SMP, nested fatal faults or all future ISA extensions.
