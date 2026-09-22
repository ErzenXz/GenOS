# BSP admission and the current single-core contract

GenOS admits exactly one hardware-designated bootstrap processor (BSP) to
kernel initialization. This closes the accidental-entry part of roadmap F5;
it does not complete the concurrency gate or provide SMP support.

## Entry and ownership

The bootloader transfers control at CPL0 in x86_64 long mode through the
existing SysV64 `_start(BootInfo)` ABI, with a valid stack and boot argument.
There is no application-processor (AP) reset-mode trampoline. The guard is a
defense at this existing kernel entry boundary, not a replacement for those
firmware/ABI prerequisites.

`_start` is a small admission shim, with the large initialization frame kept in
a separate non-inlined function. `arch::claim_boot_cpu()` runs before serial
initialization, reading boot information, constructing descriptor tables or
initializing the frame allocator. It clears IF and checks the maximum CPUID
basic leaf before querying leaf 1. Both the MSR and APIC feature bits must be
present before it reads `IA32_APIC_BASE` (MSR 0x1b). The BSP flag at bit 8 must
be set. A missing feature or non-BSP role fails closed; no unsupported MSR is
read and no APIC register is written by production admission code. The role
and feature definitions follow the [Intel architecture manuals](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html).

An irreversible atomic transition grants boot ownership once. A rejected AP
does not consume the BSP's claim. Reentering `_start` on the BSP is rejected
even before table construction. `arch::init()` independently checks the BSP
role and advances the claimed phase exactly once before touching GDT, TSS,
IDT or their static stacks. Calling it before admission or a second time
halts before those writes. Rejection leaves IF clear and enters the halt
loop silently: a rejected CPU must not initialize or race the BSP's UART.

After admission, a naked SysV64 transfer switches to a 2 MiB page-aligned kernel
BSS stack before calling `kernel_main`. RDI carries BootInfo unchanged, RSP is
16-byte aligned before CALL, and the transfer cannot return. The firmware stack
is used only for the small admission path. This matters on the four-CPU reference
VM: the former large `_start` prologue crossed a firmware stack guard before its
first Rust statement. Rejected entrants never switch to the shared kernel stack.
The stack stays reserved with the kernel ELF. After image protection, it receives
dedicated inaccessible pages on both sides, as do the privilege, IRQ and emergency
stacks. High-water measurements and the pre-installation boot window remain open.
See [kernel stack guards](KERNEL_STACKS.md) and the original
[ADR 0005](adr/0005-owned-kernel-boot-stack.md).

Successful initialization emits exactly one diagnostic:

```text
SMP_DISABLED policy=bsp-only active_cpus=1 initial_apic_id=0 cpuid_max_logical_per_package=4
```

The final number is CPUID leaf 1's package capacity, gated by the HTT feature;
zero means that legacy field is unavailable or zero. It is not the number of
discovered, parked, online or usable machine CPUs. The initial 8-bit APIC ID
is diagnostic only; identity is never inferred from APIC ID zero. GenOS does
not enumerate ACPI processor tables or start APs. A VM with four CPUs still
runs GenOS on one CPU.

## Interrupt and shared-state rules

The following rules describe the existing design and requirements for work
within it. They are not a claim that every shared global has been audited.

- Bootstrap descriptor-table writes run with IF clear after the two admission
  gates. IRQ/syscall gates are installed before interrupts are enabled; the
  IDT becomes read-only before normal IRQ-driven work begins.
- `disable_interrupts()` and `enable_interrupts()` use compiler memory
  barriers as well as CLI/STI. Their assembly does not claim to preserve IF.
  A scoped caller must save the prior IF state and restore it only if it was
  previously set. Unconditionally enabling IRQs in an inner scope is invalid.
- Masking local IRQs does not mask NMI, machine-check, synchronous faults or
  another CPU. A critical section is not an SMP lock. The admission atomics
  protect only boot phases and confer no synchronization on later globals.
- Interrupt gates clear IF on entry. The timer can save/preempt an armed
  userspace process, while ordinary kernel work is not a second scheduled
  kernel thread. `run_slice` installs `CURRENT_PROCESS` and switches CR3 with
  local IRQs masked, enters the saved user context, then returns to the kernel
  root and clears current-process state before restoring the prior IF state.
- The VirtIO MSI-X handler records atomic readiness and acknowledges the
  interrupt. Descriptor validation, buffer transfer and protocol state
  mutation remain coordinator work. An IRQ handler must not wait on a lock
  held by interrupted normal code, allocate unbounded work, or block.
- Emergency entries have dedicated guarded stacks. Repeated entry on the same
  IST, NMI/fatal nesting and full nested-interrupt behavior remain unestablished;
  such paths must not rely on ordinary IRQ masking. The current single-CPU
  floating-point/SIMD ownership is explicitly bounded by the
  [CPU state policy](CPU_STATE.md).

There is no general lock hierarchy yet. Current-process and active-address-
space globals, physical allocator ownership, scheduler-local state and some
runtime/device globals still require explicit ownership, IRQ-safe access and
per-CPU abstractions. Before a second CPU can run, GenOS needs a reviewed AP
startup/parking protocol, per-CPU GDT/TSS/stacks and process state, IRQ-safe
synchronization and lock ordering, address-space sharing rules, TLB
shootdowns and delayed/nested interrupt stress proofs. None is implied by
the admission gate. See [known limitations](KNOWN_LIMITATIONS.md).

## Reproducible checks

```sh
cargo test -p kernel --lib boot_cpu
python3 -m unittest discover -s tools -p test_bsp_harness.py
python3 tools/test_bsp.py
```

The QEMU harness requires Python 3.12 or newer and clean, committed source and builds disposable
copies with the normal `cargo xtask build` path. It never changes the working
checkout or the user's persistent data disk. Each case stores the source
commit, tool versions, fixture patch, image hash, command and serial log in
its own directory under `build/bsp-evidence/`. A failed run retains an
incomplete manifest and its available logs.

The five cases are normal one-CPU and four-CPU boots through shell readiness,
recursive `_start` entry after the first successful admission, repeated
`arch::init`, and a non-BSP identity sample before first admission. The last
fixture clears only bit 8 in the value returned after the real RDMSR, leaving
the admission policy unchanged. [QEMU's TCG APIC model](https://github.com/qemu/qemu/blob/master/hw/intc/apic.c)
preserves its BSP flag on WRMSR, so this is explicitly an injected sample, not an actual AP or a
hardware role change. It does not prove an AP trampoline or startup sequence.
The negative fixtures add rejection-branch serial markers after arranging
UART access on their sole executing CPU. These markers and sample injection
are absent from production builds. Tests reject any later table initialization,
normal readiness, exception, panic, duplicate marker or QEMU reset.

Host tests separately cover missing APIC/MSR features, a missing MSR sample,
APIC ID independence, rejected entrants preserving the claim, table phase
ordering, package-field semantics and simultaneous atomic claims. The host
thread test proves one admission grant; it does not demonstrate SMP kernel
safety. Ordinary one/four-CPU boot proves the current UEFI reference path
keeps one active kernel CPU, not that all firmware or physical hardware does.
