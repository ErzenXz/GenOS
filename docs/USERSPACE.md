# GenOS userspace boundary

GenOS 0.6 contains the first hardware-enforced transition between kernel and user privilege. This document states exactly what that milestone proves and what it does not.

## Boot sequence

1. The bootloader supplies a typed UEFI memory map.
2. The kernel builds a frame allocator from page-aligned `Usable` regions. Reserved gaps, firmware boot-services memory, and the null page are never returned.
3. The kernel clones the active four-level page tables and clears the user-accessible bit from every existing mapping.
4. Only the probe's dedicated code page and four stack pages are exposed to ring 3. A page below the stack remains supervisor-only as a guard.
5. An `iretq` frame enters the probe using user code and data selectors.
6. The probe calls the DPL3 `int 0x80` gate for a ping, ABI-version query, and exit.
7. The kernel validates the syscall number and scalar arguments, returns values to the saved user frame, and handles exit by restoring the original ring-0 continuation.
8. GenOS records the completed `init` user process and continues normal desktop startup.

The QEMU smoke test requires `PAGING_READY`, `USER_CONTEXT_OK`, `USER_SYSCALL_OK`, and `USERMODE_READY`, so CI fails if the protected address space, ring-3 selectors and ranges, syscall crossing, or return to the desktop regresses.

## ABI version 1

The syscall number is passed in `rax`. Scalar arguments use `rdi`, `rsi`, `rdx`, `r10`, `r8`, and `r9`. Results are returned in `rax`.

| Number | Name | Arguments | Result |
| ---: | --- | --- | --- |
| 0 | `ping` | all zero | fixed GenOS reply value |
| 1 | `abi_version` | all zero | ABI version `1` |
| 2 | `exit` | `rdi` = status `0..255`; remaining arguments zero | terminates the current probe |

Unknown syscall numbers and invalid scalar arguments return stable negative-style error values without performing kernel work.

## Current guarantees

- User code executes with CPU privilege level 3.
- Existing kernel mappings are supervisor-only in the cloned address space.
- User access is granted page by page, not by exposing a large identity-mapped region.
- The user code page is read-only and executable; the user stack is writable and non-executable when NX is enabled.
- The user stack has a non-user-accessible guard page beneath it.
- Syscall dispatch does not trust unvalidated numbers or scalar arguments.
- A successful userspace exit returns control to the kernel and desktop boot continues.

## Current limitations

- There is one built-in boot probe, not a general executable loader.
- The scheduler's workers remain kernel-mode workloads.
- There is not yet one address space per process.
- The syscall ABI has no pointer or buffer arguments yet.
- Timer interrupts do not preempt user processes.
- Faults in a user process are not yet converted into process-local termination.
- The exit return path is single-core and supports one active user transition.

The next slice will introduce saved CPU contexts and per-process address-space ownership before expanding the syscall surface.
