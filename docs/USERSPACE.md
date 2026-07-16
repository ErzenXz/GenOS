# GenOS userspace boundary

GenOS 0.10 adds an asynchronous lifecycle, validated application output, and deterministic frame reclamation to the independently built ELF runtime. This document states exactly what the milestone proves and what it does not.

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

## Execution and lifecycle proof

At boot, GenOS creates three independent instances of `INIT.ELF` for the preemption and fault-containment proof:

1. all instances query ABI version 3 and become eligible for timer scheduling;
2. a 100 Hz PIT interrupt involuntarily preempts each process and saves its full CPU context;
3. the first instance writes to its guard page and is terminated with page-fault status 142 before performing output work;
4. the two healthy instances resume afterward, write greetings through the validated output syscall, report private values through validated copy-in, and exit with status 0.

GenOS then launches a fourth instance through the general ELF launch function and verifies preemption, output, private memory, normal exit, and reclamation. A separate lifecycle probe starts a normal asynchronous process and a persistent held process. The normal process outputs text, exits, and is reaped. The held process is preempted, killed with status 137, and reaped. Both release ten frames: two ELF pages, four stack pages, three user page-table pages, and one CR3 root.

The QEMU smoke test requires markers for asynchronous exit, userspace output, kill, wait/reap, frame reclamation, fault containment, and the long-lived desktop. Recycled roots are visibly reused by later processes in the serial proof.

## Desktop lifecycle

- `run init` reserves a user task, constructs a fresh process, and returns immediately. The desktop loop schedules one userspace slice on later ticks.
- `run init hold` launches the same ELF with a persistent token. After its greeting, it remains runnable until killed.
- `ps` shows the user task alongside system and kernel-worker records.
- `kill PID` terminates a live userspace task with status 137 and immediately releases its address space.
- `wait PID` is non-blocking. It reports “still running” for a live process; after exit, fault, or kill it returns the retained result and frees the process-manager slot.

Completed task history remains in the task registry even after the heavier process resources have been reclaimed. When the bounded task table is full, a later launch may reuse a terminal history slot with a new PID.

## ABI version 3

The syscall number is passed in `rax`. Scalar arguments use `rdi`, `rsi`, `rdx`, `r10`, `r8`, and `r9`. Results are returned in `rax`.

| Number | Runtime function | Arguments | Result |
| ---: | --- | --- | --- |
| 0 | `ping` | all zero | fixed GenOS reply value |
| 1 | `abi_version` | all zero | ABI version `3` |
| 2 | `exit` | status `0..255`; remaining arguments zero | terminates the current process instance |
| 3 | `yield_now` | all zero | cooperatively returns to the kernel scheduler |
| 4 | `report_u64` | owned user address and length `8` | validated value copied from user memory |
| 5 | `write` | mapped user address and length `1..80` | sanitized text length after copy-in |

The output path validates the whole range against the userspace window, translates every byte through the owning address space, rejects unmapped holes, and replaces control or non-ASCII bytes before the shell sees them. The application uses runtime functions instead of handwritten assembly. Cooperative yield remains available for ABI compatibility, but the execution proof relies on timer preemption.

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
- Shell launch is asynchronous and keeps the desktop loop responsive between userspace slices.
- Normal exit, user fault, and explicit kill all reclaim the process's mapped pages, private page-table branch, and CR3 root.
- Freed physical frames are rejected on double-free and reused before new bump allocation.
- Bounded userspace text reaches the shell only after per-byte mapping validation and sanitization.
- Completed processes retain exit, fault, or kill status until `wait` reaps their manager slot.

## Current limitations

- `INIT.ELF` is the only packaged userspace program.
- The immutable initrd ELF is registered directly with the loader and is not copied into the small writable session VFS.
- `wait` is observational rather than blocking; there is no parent/child ownership model yet.
- The process manager has four slots. A terminal process occupies one until `wait` reaps it.
- The recycled-frame pool is intentionally bounded to 256 frames; this milestone does not provide a general coalescing physical-memory allocator.
- There is no userspace sleep, blocking input, heap allocator, filesystem API, IPC, or window API.
- Output is a bounded text syscall, not file-descriptor-based standard I/O.
- The transition state is single-core and supports one active user process at a time.

The next slice is blocking userspace sleep/wake, parent-child wait semantics, and a bounded pipe or message-channel primitive so multiple ELF processes can coordinate without kernel-global state.
