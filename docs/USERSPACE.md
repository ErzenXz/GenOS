# GenOS userspace boundary

GenOS 0.12 adds versioned structured copy-out and a blocking VFS read path to the independently built ELF runtime. This document states exactly what the milestone proves and what it does not.

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
- a writable data mapping at the ABI data address with space for the stable `UserProcessHeader` and application buffers.

Every accepted page receives a newly allocated zeroed physical frame. File bytes are copied into those frames, remaining memory is left zeroed, and page-table permissions come directly from the validated segment flags. Stacks are mapped separately with an unmapped guard page.

## Execution and lifecycle proof

At boot, GenOS creates three independent instances of `INIT.ELF` for the preemption and fault-containment proof:

1. all instances query ABI version 5 and become eligible for timer scheduling;
2. a 100 Hz PIT interrupt involuntarily preempts each process and saves its full CPU context;
3. the first instance writes to its guard page and is terminated with page-fault status 142 before performing output work;
4. the two healthy instances resume afterward, write greetings through the validated output syscall, report private values through validated copy-in, and exit with status 0.

GenOS then launches a fourth instance through the general ELF launch function and verifies preemption, output, private memory, normal exit, and reclamation. A separate lifecycle probe starts a normal asynchronous process and a persistent held process. The normal process outputs text, exits, and is reaped. The held process is preempted, killed with status 137, and reaped. Both release ten frames: two ELF pages, four stack pages, three user page-table pages, and one CR3 root.

The probe then creates an owned parent-child pair. Each has its own CR3 root. The parent blocks while waiting on the exact child PID. The child blocks on a three-tick sleep deadline, wakes, places a value in the parent's bounded inbox, and exits with status 7. Child termination injects that status into the parent's saved `rax` and returns the parent to `Ready`; the parent's subsequent receive removes the queued value before it exits with status 0. Both address spaces are reclaimed and both terminal records are reaped.

Finally, a file-mode process requests `UserSystemInfo` through structured copy-out and issues `read_file` for `/README.TXT`. The kernel validates both user ranges, copies the path in, records the destination, and blocks the process. A scheduler poll confirms that no userspace slice runs while the request is outstanding. The VFS supplies 54 bytes on a later step; the kernel revalidates request identity and the destination mapping, copies the bytes into the process-owned physical frame, places `54` in saved `rax`, and wakes the process. Ring 3 compares every byte with the expected file before exiting with status 0.

The QEMU smoke test requires markers for structured copy-out, file block/wake, exact content verification, sleep/block/wake, owned child wait/wake, message send/receive, frame reclamation, fault containment, and the long-lived desktop. Recycled roots are visibly reused by later processes in the serial proof.

## Desktop lifecycle

- `run init` reserves a user task, constructs a fresh process, and returns immediately. The desktop loop schedules one userspace slice on later ticks.
- `run init hold` launches the same ELF with a persistent token. After its greeting, it remains runnable until killed.
- `run init sleep` blocks the process for three scheduler ticks and prints a second line only after its deadline wakeup.
- `run init file` copies out system metadata, blocks on `/README.TXT`, verifies the returned bytes, and prints success after wakeup.
- `run pair` reserves two task records and launches the parent-child coordination proof. Task Manager exposes their `waiting`, `sleeping`, `ready`, and terminal transitions.
- `ps` shows the user task alongside system and kernel-worker records.
- `kill PID` terminates a live userspace task with status 137 and immediately releases its address space.
- `wait PID` is non-blocking. It reports “still running” for a live process; after exit, fault, or kill it returns the retained result and frees the process-manager slot.

Completed task history remains in the task registry even after the heavier process resources have been reclaimed. When the bounded task table is full, a later launch may reuse a terminal history slot with a new PID.

The shell's `wait PID` remains an observational reap command for operators. ABI `wait_child` is the blocking primitive used by a Ring 3 parent; the two operations intentionally serve different callers.

## ABI version 5

The syscall number is passed in `rax`. Scalar arguments use `rdi`, `rsi`, `rdx`, `r10`, `r8`, and `r9`. Results are returned in `rax`.

| Number | Runtime function | Arguments | Result |
| ---: | --- | --- | --- |
| 0 | `ping` | all zero | fixed GenOS reply value |
| 1 | `abi_version` | all zero | ABI version `5` |
| 2 | `exit` | status `0..255`; remaining arguments zero | terminates the current process instance |
| 3 | `yield_now` | all zero | cooperatively returns to the kernel scheduler |
| 4 | `report_u64` | owned user address and length `8` | validated value copied from user memory |
| 5 | `write` | mapped user address and length `1..80` | sanitized text length after copy-in |
| 6 | `sleep` | deadline delta `1..10000` ticks | `0` after the scheduler wakes the saved context |
| 7 | `send` | target PID `1..255`, fixed-width value | `0`, or a bounded error if the target is unavailable or its inbox is full |
| 8 | `receive` | all zero | next fixed-width inbox value; blocks if the inbox is empty |
| 9 | `wait_child` | child PID `1..255` | child exit status; blocks while an owned child remains live |
| 10 | `system_info` | writable address and exact structure size `40` | copies `UserSystemInfo` and returns `40` |
| 11 | `read_file` | path address/length and writable output address/capacity | blocks, then returns copied byte count or a bounded error |

The output path validates the whole range against the userspace window, translates every byte through the owning address space, rejects unmapped holes, and replaces control or non-ASCII bytes before the shell sees them. The application uses runtime functions instead of handwritten assembly. Cooperative yield remains available for ABI compatibility, but the execution proof relies on timer preemption.

Blocking syscalls copy the normalized interrupt frame into the process context and return to Ring 0. Sleeping and waiting processes are excluded from runnable selection. A wakeup writes the syscall result into saved `rax` before changing the state back to `Ready`, so execution continues immediately after the original `int 0x80` instruction.

Each managed process owns a four-value FIFO inbox. `send` either wakes a receiver directly or appends at the tail; it never allocates and never silently overwrites an older value. `wait_child` accepts only a live or retained process whose recorded parent PID matches the caller. These policies keep the first IPC contract small and deterministic.

`UserSystemInfo` is a `repr(C)` structure of five `u64` fields: ABI version, page size, timer frequency, message capacity, and maximum file-read size. Its size and alignment are tested. `UserProcessHeader` fixes the kernel-owned token and preemption words at offsets 0 and 8; those offsets are also tested so adding application data cannot silently break preemption again.

`read_file` accepts paths of 1–64 ASCII bytes and output capacities of 1–128 bytes. The path must be absolute and use only letters, numbers, `/`, `.`, `_`, or `-`. The output must remain inside the process's writable data page and translate to the physical frame owned by that process. Completion checks the original task ID, Ring 3 PID, path, and capacity before copying. Reads larger than the supplied capacity are truncated like a conventional bounded read.

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
- Sleeping and waiting processes cannot consume userspace scheduler slices.
- Sleep deadlines use saturating tick arithmetic and a bounded `1..10000` duration.
- A child exit wakes a blocked parent and returns the exact eight-bit exit status through the saved syscall frame.
- Only the recorded parent PID can use `wait_child` for a process.
- Each process inbox is a four-value FIFO; full or unavailable delivery returns an error instead of overwriting data.
- Structured copy-out is limited to the process-owned writable data mapping and revalidates every translated byte.
- The process token and preemption counter have tested, shared ABI offsets.
- File reads leave the process non-runnable until the VFS completion path injects a result into saved `rax`.
- A pending file completion must match the original task ID, Ring 3 PID, path, and capacity.

## Current limitations

- `INIT.ELF` is the only packaged userspace program.
- The immutable initrd ELF is registered directly with the loader and is not copied into the small writable session VFS.
- Shell `wait PID` is observational; blocking semantics are available only to a userspace parent through `wait_child`.
- The process manager has four slots. A terminal process occupies one until `wait` reaps it.
- The recycled-frame pool is intentionally bounded to 256 frames; this milestone does not provide a general coalescing physical-memory allocator.
- Messages carry one `u64`; there are no byte streams, endpoint handles, permissions beyond process availability, or multi-producer fairness guarantees yet.
- The file API is read-only, path-based, capped at 128 bytes, and backed only by the session RAM VFS. There are no handles, offsets, directories, writes, or persistent storage yet.
- The desktop holds one pending VFS request because it schedules at most one userspace slice and services one completion per tick.
- There is no blocking input, heap allocator, or userspace window API.
- Output is a bounded text syscall, not file-descriptor-based standard I/O.
- The transition state is single-core and supports one active user process at a time.

The next slice is capability-bearing file handles with offsets and stat/close, followed by bounded writes and blocking input events built on the same saved-context wakeup model.
