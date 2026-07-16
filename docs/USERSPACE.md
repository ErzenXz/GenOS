# GenOS userspace boundary

GenOS 0.8 extends the hardware-enforced user boundary with timer preemption and process-local fault containment. This document states exactly what that milestone proves and what it does not.

## Boot sequence

1. The bootloader supplies a typed UEFI memory map.
2. The kernel builds a frame allocator from page-aligned `Usable` regions. Reserved gaps, firmware boot-services memory, and the null page are never returned.
3. The kernel clones the active four-level page tables and clears the user-accessible bit from every existing mapping.
4. Three process instances receive separate PML4 roots. Each root shares the supervisor template but owns an otherwise unused lower-half slot.
5. Each root maps private physical code, data, and stack frames at the same user virtual addresses. A page below each stack remains unmapped as a guard.
6. An `iretq` frame enters each process using user code and data selectors.
7. Every process calls the DPL3 `int 0x80` gate, writes a distinct private token, and waits in userspace without yielding.
8. A successful ABI query arms that process for scheduling. This prevents machine-speed differences from consuming a quantum before the boot probe reaches its validated starting point.
9. A 100 Hz PIT interrupt captures every general-purpose register plus `rip`, `rsp`, selectors, and flags. The IRQ acknowledges the PIC and returns to the Ring 0 scheduler instead of the interrupted process.
10. The scheduler switches CR3 and later resumes each process at the interrupted instruction. A private per-process probe flag makes this preemption proof deterministic without a cooperative syscall.
11. The first process writes to its unmapped guard page. The page-fault handler recognizes a Ring 3 frame, records vector 14 and exit status 142, and returns to the scheduler instead of halting the kernel.
12. The two healthy processes continue after that fault, report their private values through validated copy-in, and exit normally.
13. GenOS verifies distinct roots and frames, preserved private values, three timer preemptions, fault-first completion order, two later successful exits, and a live desktop.

The QEMU smoke test requires preemption, process-local termination, validated-copy, isolation, and long-lived desktop markers. CI therefore fails if timer context capture, CR3 ownership, fault containment, independent exit, or return to the desktop regresses.

## ABI version 2

The syscall number is passed in `rax`. Scalar arguments use `rdi`, `rsi`, `rdx`, `r10`, `r8`, and `r9`. Results are returned in `rax`.

| Number | Name | Arguments | Result |
| ---: | --- | --- | --- |
| 0 | `ping` | all zero | fixed GenOS reply value |
| 1 | `abi_version` | all zero | ABI version `2` |
| 2 | `exit` | `rdi` = status `0..255`; remaining arguments zero | terminates the current process instance |
| 3 | `yield` | all zero | saves the current context and returns to the kernel scheduler |
| 4 | `report` | `rdi` = user address; `rsi` = `8`; remaining arguments zero | validated 64-bit value copied from owned user memory |

The cooperative `yield` call remains available for ABI compatibility, but the 0.8 proof does not use it. Unknown syscall numbers and invalid scalar arguments return stable negative-style error values without performing kernel work.

## Interrupt and fault contract

Timer IRQs and syscalls normalize their saved registers to the same `UserContext` layout. When IRQ0 interrupts Ring 3, the kernel copies that frame into the active process, switches back to the protected kernel root, and schedules another runnable process. Kernel and desktop timer interrupts follow the ordinary IRQ return path.

CPU faults include an error code before the interrupt-return frame. The fault stub passes the saved code selector to Rust so the handler can distinguish Ring 3 failures from kernel failures. A user page fault terminates only the current process. A kernel page fault still emits fatal diagnostics and halts because continuing after kernel corruption would be unsafe.

The current fault probe produces error code `0x6`: a user-mode write to a non-present page. Its `cr2` value must equal the process stack-guard address.

## Current guarantees

- All three process instances execute with CPU privilege level 3.
- Existing kernel mappings remain supervisor-only in every process root.
- Each process owns a distinct CR3 root and distinct user physical frames.
- User code is read-only and executable; data and stack pages are writable and non-executable when NX is enabled.
- The user stack has an unmapped guard page beneath it.
- PIT interrupts preempt userspace without a syscall and preserve the complete user context.
- A Ring 3 page fault becomes terminal status for only the active process.
- Healthy processes resume and finish after a peer process faults.
- User pointers are range checked, translated through the owning root, and matched to the expected physical frame before access.
- Returning to the kernel after preemption, fault, and exit preserves the later desktop interrupt path.

## Current limitations

- The processes are built-in instances of one embedded code page, not dynamically loaded executables.
- The scheduler's shell-created workers remain kernel-mode workloads.
- There is no userspace sleep, wake, blocking I/O, or priority model yet.
- Address-space frames are not yet reclaimed after exit or fault.
- Only page fault, general protection fault, and double fault have dedicated diagnostic handlers.
- The transition state is single-core and supports one active user process at a time.

The next slice is a validated userspace ELF loader and initial runtime crate, followed by dynamic launch and lifecycle controls.
