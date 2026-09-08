# ADR-0005: Admit the BSP before switching to a kernel-owned boot stack

- **Status:** Proposed
- **Date:** 2026-09-08
- **Decision owners:** GenOS maintainers
- **Related roadmap gate:** F1, F5, F6
- **Supersedes:** None

## Context

The four-CPU QEMU reference boot reached the ELF entry but triple-faulted before
printing `GenOS kernel entered`. QEMU exception tracing identified a write fault
at `_start + 0x15`: the compiler's stack-probe loop crossed a firmware guard page
at `0x1fdff3b0`. The development entry frame reserved about 228 KiB for runtime
and filesystem construction before executing CPU admission. One-CPU firmware
happened to tolerate this; its success did not establish a stack-size contract.

## Decision and invariants

Keep `_start` as a small admission shim on the valid firmware-provided stack.
Only the hardware BSP may acquire the irreversible boot claim. Rejection halts
without switching stacks, initializing the UART, or touching descriptor tables.

After admission, a naked SysV64 function switches RSP to the top of a 2 MiB,
page-aligned kernel BSS allocation, clears RBP, and calls a non-inlined
`kernel_main`. RDI preserves the boot-lifetime immutable BootInfo pointer; RSP
is aligned to 16 bytes before CALL. Returning from initialization is invalid and
ends in UD2. IF remains clear through admission and stack transfer.

The kernel ELF reserves the stack's physical pages. They are excluded from the
frame allocator and later receive supervisor writable/non-executable kernel data
permissions. The stack has one owner for the entire boot and is never reclaimed.
The existing boot ABI and userspace ABI are unchanged.

## Alternatives

Keeping the firmware stack leaves a demonstrated configuration-dependent fault.
Merely reducing local variables does not give future code a defined stack budget.
Allocating a stack through firmware would add a boot ABI allocation/lifetime
contract. Static kernel storage provides a bounded, inspectable initial contract
without depending on the normal allocator or changing BootInfo.

## Consequences and residual limits

The image reserves 2 MiB of additional BSS memory. The small admission function
still requires the firmware's valid SysV64 stack and execution environment.
A dedicated overflow guard, stack high-water accounting, NMI/fatal nesting,
per-CPU stacks, AP trampolines and SMP remain open. Local IRQ masking does not
protect against NMI or a second CPU; the irreversible admission gate is what
prevents a second caller from resetting the shared stack.

## Compatibility, migration and rollback

Rebuild the kernel image normally; there is no data migration or bootloader ABI
change. Reverting the shim and stack together restores the earlier entry but also
reintroduces the reproduced four-CPU firmware-stack failure. Never move the stack
switch before admission or inline the large initialization function into `_start`.

## Verification

`python3 tools/test_bsp.py` boots one- and four-CPU VMs and injects repeated entry,
repeated table initialization, and a non-BSP identity sample. Required markers
must appear exactly once; a rejected entry must halt before later initialization.
The four-CPU normal case is the system regression for the firmware guard fault.
`cargo xtask test-release` checks both normal build profiles, and `cargo xtask test`
checks validation startup, storage, networking, memory rollback and CPU protections.
Disassembly of `_start` must show a small frame with no large stack-probe loop.
Physical hardware and stack exhaustion are not established by these checks.

See [the single-core contract](../SINGLE_CORE.md) and
[normal/validation policy](../BOOT_MODES.md).
