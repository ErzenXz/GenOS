# GenOS userspace boundary

GenOS 0.7 extends the hardware-enforced user boundary into two resumable processes with private address spaces. This document states exactly what that milestone proves and what it does not.

## Boot sequence

1. The bootloader supplies a typed UEFI memory map.
2. The kernel builds a frame allocator from page-aligned `Usable` regions. Reserved gaps, firmware boot-services memory, and the null page are never returned.
3. The kernel clones the active four-level page tables and clears the user-accessible bit from every existing mapping.
4. Each process receives a new PML4 root that shares the supervisor template but owns an otherwise unused lower-half slot.
5. Each root maps private physical code, data, and stack frames at the same user virtual addresses. A page below each stack remains unmapped as a guard.
6. An `iretq` frame enters process A and then process B using user code and data selectors.
7. Both processes call the DPL3 `int 0x80` gate, write distinct private tokens, and yield.
8. The kernel saves every general-purpose register plus `rip`, `rsp`, selectors, and flags, switches CR3, and later resumes both processes after the yielding instruction.
9. A report syscall validates the user buffer, translates it through the current process's page tables, confirms physical ownership, and only then copies the value.
10. Both processes exit independently. GenOS verifies distinct roots, distinct physical data frames, intact private values, unmapped guards, and successful exits before starting the desktop.

The QEMU smoke test requires address-space, context-resume, validated-copy, isolation, and long-lived desktop markers. CI therefore fails if CR3 ownership, saved contexts, user-buffer validation, independent exit, or return to the desktop regresses.

## ABI version 2

The syscall number is passed in `rax`. Scalar arguments use `rdi`, `rsi`, `rdx`, `r10`, `r8`, and `r9`. Results are returned in `rax`.

| Number | Name | Arguments | Result |
| ---: | --- | --- | --- |
| 0 | `ping` | all zero | fixed GenOS reply value |
| 1 | `abi_version` | all zero | ABI version `2` |
| 2 | `exit` | `rdi` = status `0..255`; remaining arguments zero | terminates the current process instance |
| 3 | `yield` | all zero | saves the current context and returns to the kernel scheduler |
| 4 | `report` | `rdi` = user address; `rsi` = `8`; remaining arguments zero | validated 64-bit value copied from owned user memory |

Unknown syscall numbers and invalid scalar arguments return stable negative-style error values without performing kernel work.

## Current guarantees

- Both processes execute with CPU privilege level 3.
- Existing kernel mappings remain supervisor-only in every process root.
- Each process owns a distinct CR3 root and distinct user physical frames.
- User code is read-only and executable; data and stack pages are writable and non-executable when NX is enabled.
- The user stack has an unmapped guard page beneath it.
- Yield preserves the full general-purpose and interrupt-return context.
- User pointers are range checked, translated through the owning root, and matched to the expected physical frame before access.
- Both successful exits return control to the kernel and desktop boot continues.

## Current limitations

- There are two built-in process instances, not a general executable loader.
- The scheduler's workers remain kernel-mode workloads.
- Userspace switching is cooperative through `yield`; timer interrupts do not preempt processes yet.
- Address-space frames are not yet reclaimed after exit.
- Faults in a user process are not yet converted into process-local termination.
- The transition state is single-core and supports one active user process at a time.

The next slice will connect timer preemption and user page faults to the process lifecycle, then introduce a userspace ELF loader.
