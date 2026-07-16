# GenOS userspace boundary

GenOS 0.9 replaces the kernel-embedded user code page with an independently built ELF executable and an initial userspace runtime. This document states exactly what the milestone proves and what it does not.

## Build and packaging pipeline

1. `userspace/runtime` builds as a `no_std` library and owns the initial `int 0x80` syscall wrappers.
2. `userspace/init` builds as a separate static `x86_64` executable using the dedicated `userspace` Cargo profile.
3. Its linker script emits an RX text segment at the GenOS user entry and a separate RW data segment. The link fails if either segment grows beyond one page.
4. `xtask` builds the userspace executable before the kernel and packages it into the boot initrd as `INIT.ELF`.
5. The kernel locates `INIT.ELF` by name. A missing or invalid image stops the boot before any Ring 3 transition.

The kernel binary no longer contains a `.usertext` payload. Userspace behavior comes from the ELF bytes supplied through the boot filesystem.

## ELF validation and mapping

The bounded parser accepts only little-endian ELF64 executable files for x86_64. It validates the ELF and program-header sizes, caps the program-header count, checks every offset and length with overflow-safe arithmetic, requires at least one loadable segment, and rejects truncated file data.

Before allocating user pages, the process loader additionally requires:

- page-aligned segment virtual addresses and at least page alignment;
- readable load segments with no unknown permission bits;
- write and execute permissions are never both present on one segment;
- segment memory ranges entirely inside the reserved user-image window;
- no overlapping virtual pages;
- an entry point inside an executable segment;
- a writable data mapping at the ABI data address with space for the process token and preemption counter.

Every accepted page receives a newly allocated zeroed physical frame. File bytes are copied into those frames, remaining memory is left zeroed, and page-table permissions come directly from the validated segment flags. Stacks are mapped separately with an unmapped guard page.

## Execution proof

At boot, GenOS creates three independent instances of `INIT.ELF` for the preemption and fault-containment proof:

1. all instances query ABI version 2 and become eligible for timer scheduling;
2. a 100 Hz PIT interrupt involuntarily preempts each process and saves its full CPU context;
3. the first instance writes to its guard page and is terminated with page-fault status 142;
4. the two healthy instances resume afterward, report private values through validated copy-in, and exit with status 0.

GenOS then launches a fourth instance through the general ELF launch function, verifies preemption, private memory, and exit status, and records it as `init-elf` in the task table. The QEMU smoke test requires the ELF validation, mapping, dynamic-launch, preemption, fault-containment, and long-lived desktop markers.

From the desktop shell, `run init` invokes the same launch function again. A successful command creates another CR3 root, maps new segment and stack frames, enters Ring 3, receives a timer preemption, exits, appends completed task history, and returns control to the desktop.

## ABI version 2

The syscall number is passed in `rax`. Scalar arguments use `rdi`, `rsi`, `rdx`, `r10`, `r8`, and `r9`. Results are returned in `rax`.

| Number | Runtime function | Arguments | Result |
| ---: | --- | --- | --- |
| 0 | `ping` | all zero | fixed GenOS reply value |
| 1 | `abi_version` | all zero | ABI version `2` |
| 2 | `exit` | status `0..255`; remaining arguments zero | terminates the current process instance |
| 3 | `yield_now` | all zero | cooperatively returns to the kernel scheduler |
| 4 | `report_u64` | owned user address and length `8` | validated value copied from user memory |

The 0.9 application uses the runtime functions instead of handwritten assembly. Cooperative yield remains available for ABI compatibility, but the execution proof relies on timer preemption.

## Interrupt safety

Timer IRQs and syscalls normalize their saved registers to the same `UserContext` layout. The kernel disables interrupts around the active-process pointer and CR3 transition, then restores the caller's prior interrupt state after returning to Ring 0. This matters for shell-triggered launches because the desktop normally runs with hardware interrupts enabled.

Only Ring 3 page faults and general-protection faults can become process-local termination. Double faults and all kernel faults remain fatal. Continuing after suspected kernel corruption would be unsafe.

## Current guarantees

- Userspace is compiled and linked independently from the kernel.
- ELF metadata and every load segment are validated before mapping or execution.
- Executable pages are not writable, and writable data pages are not executable.
- Every process instance owns a distinct CR3 root and distinct physical image, data, and stack frames.
- Timer preemption preserves the complete user context without a cooperative syscall.
- A Ring 3 page fault terminates only the active process.
- Healthy processes and the desktop remain alive after a peer faults.
- Both boot code and the shell can launch fresh instances from the packaged ELF image.
- User pointers are range checked, translated through the owning root, and matched to the expected physical frame before access.

## Current limitations

- `INIT.ELF` is the only packaged userspace program.
- Shell launch is synchronous; the command waits for the short-lived program to exit.
- There is no asynchronous start, live-process inspection, kill, wait, or parent/child model for userspace yet.
- The immutable initrd ELF is registered directly with the loader and is not copied into the small writable session VFS.
- Address-space and process frames are not reclaimed after exit or fault, so repeated launches are intentionally bounded by available memory and task history.
- There is no userspace sleep, blocking I/O, standard output, heap allocator, or window API.
- The transition state is single-core and supports one active user process at a time.

The next slice is an asynchronous userspace process manager with wait/kill lifecycle controls, frame reclamation, and a small output syscall so ELF applications can interact with the shell.
