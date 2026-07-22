# GenOS userspace boundary

GenOS 0.15 adds blocking keyboard and pointer events, filtered delivery, and deterministic input ownership to the independently built ELF runtime. This document states exactly what the milestone proves and what it does not.

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

1. all instances query ABI version 8 and become eligible for timer scheduling;
2. a 100 Hz PIT interrupt involuntarily preempts each process and saves its full CPU context;
3. the first instance writes to its guard page and is terminated with page-fault status 142 before performing output work;
4. the two healthy instances resume afterward, write greetings through the validated output syscall, report private values through validated copy-in, and exit with status 0.

GenOS then launches a fourth instance through the general ELF launch function and verifies preemption, output, private memory, normal exit, and reclamation. A separate lifecycle probe starts a normal asynchronous process and a persistent held process. The normal process outputs text, exits, and is reaped. The held process is preempted, killed with status 137, and reaped. Both release ten frames: two ELF pages, four stack pages, three user page-table pages, and one CR3 root.

The probe then creates an owned parent-child pair. Each has its own CR3 root. The parent blocks while waiting on the exact child PID. The child blocks on a three-tick sleep deadline, wakes, places a value in the parent's bounded inbox, and exits with status 7. Child termination injects that status into the parent's saved `rax` and returns the parent to `Ready`; the parent's subsequent receive removes the queued value before it exits with status 0. Both address spaces are reclaimed and both terminal records are reaped.

Finally, a file-mode process requests `UserSystemInfo` through structured copy-out and opens `/README.TXT`. The open request blocks while the desktop VFS resolves a regular file; completion installs an opaque, read-only capability in the calling process's four-slot handle table. Ring 3 copies out `UserFileStat`, reads 17 bytes, confirms that `stat_handle` now reports offset 17, then reads the remaining 37 bytes through the same handle. The kernel derives the path and offset from the capability rather than trusting userspace. Each read blocks, and a scheduler poll confirms that no userspace slice runs while the request is outstanding.

The application compares all 54 bytes with the expected file, closes the handle, and proves that a subsequent read returns `USER_ERROR_INVALID_ARGUMENT`. The lifecycle probe also submits forged open and read completions before the valid completions and requires both to be rejected. Normal exit, fault, and kill revoke every handle still owned by the process.

A second file-mode process then requests read/write authority for `/README.TXT`; the kernel rejects it before issuing a VFS request because application mutation is limited to the case-sensitive `/USER/` prefix. The process opens `/USER/APP.TXT` with read/write rights, causing the VFS service to create an empty file. It writes 13 and then 14 bytes through two blocking requests. Each payload is copied completely from mapped userspace into a fixed 128-byte kernel buffer before the process becomes non-runnable. Successful completion advances the capability offset and size to 13 and then 27.

After close, the application reopens the file read-only. A write through that narrower capability returns `USER_ERROR_INVALID_ARGUMENT` without reaching the VFS. The application reads back and verifies all 27 bytes, closes the handle, exits with status 0, and releases its ten address-space frames. The lifecycle probe submits a forged write offset before the real completion and requires rejection without changing authority or offset.

The probe next starts an input-mode process. After two bounded output calls it invokes `wait_input` with the keyboard mask; the process becomes `Waiting`, retains its writable event address, and cannot consume another scheduler slice. A synthetic pointer movement is presented first and deliberately does not wake the keyboard-only waiter. Pointer handling therefore remains available to the desktop rather than being stolen by an unrelated subscription.

While the first process still owns the one-shot input channel, a second input-mode process attempts the same wait. It receives `USER_ERROR_UNAVAILABLE`, reports that the channel is busy, exits normally, and releases its address space. The kernel then converts key `G` into the stable 32-byte `UserInputEvent`, revalidates the first process's private writable range, copies the event, writes `32` into saved `rax`, and returns that process to `Ready`. Ring 3 verifies the kind, code, printable value, and reserved field before reporting the exact key and exiting. Both task records are reaped and both ten-frame address spaces are reclaimed.

The QEMU smoke test requires markers for structured copy-out, file block/wake, exact content verification, input block/filter/ownership/wake, sleep/block/wake, owned child wait/wake, message send/receive, frame reclamation, fault containment, and the long-lived desktop. Recycled roots are visibly reused by later processes in the serial proof.

## Desktop lifecycle

- `run init` reserves a user task, constructs a fresh process, and returns immediately. The desktop loop schedules one userspace slice on later ticks.
- `run init hold` launches the same ELF with a persistent token. After its greeting, it remains runnable until killed.
- `run init sleep` blocks the process for three scheduler ticks and prints a second line only after its deadline wakeup.
- `run init file` copies out system metadata, opens `/README.TXT`, verifies stat and offset changes across two blocking reads, closes the handle, and proves stale reuse fails.
- `run init write` proves protected-path denial, creates `/USER/APP.TXT`, performs two bounded writes, checks stat, closes and reopens read-only, proves write denial, and verifies exact read-back.
- `run init input` waits for one printable keyboard event. Matching input is routed to the application instead of the shell, copied into private memory, reported, and consumed once.
- `run pair` reserves two task records and launches the parent-child coordination proof. Task Manager exposes their `waiting`, `sleeping`, `ready`, and terminal transitions.
- `ps` shows the user task alongside system and kernel-worker records.
- `kill PID` terminates a live userspace task with status 137 and immediately releases its address space.
- `wait PID` is non-blocking. It reports “still running” for a live process; after exit, fault, or kill it returns the retained result and frees the process-manager slot.

Completed task history remains in the task registry even after the heavier process resources have been reclaimed. When the bounded task table is full, a later launch may reuse a terminal history slot with a new PID.

The shell's `wait PID` remains an observational reap command for operators. ABI `wait_child` is the blocking primitive used by a Ring 3 parent; the two operations intentionally serve different callers.

## ABI version 8

The syscall number is passed in `rax`. Scalar arguments use `rdi`, `rsi`, `rdx`, `r10`, `r8`, and `r9`. Results are returned in `rax`.

| Number | Runtime function | Arguments | Result |
| ---: | --- | --- | --- |
| 0 | `ping` | all zero | fixed GenOS reply value |
| 1 | `abi_version` | all zero | ABI version `8` |
| 2 | `exit` | status `0..255`; remaining arguments zero | terminates the current process instance |
| 3 | `yield_now` | all zero | cooperatively returns to the kernel scheduler |
| 4 | `report_u64` | owned user address and length `8` | validated value copied from user memory |
| 5 | `write` | mapped user address and length `1..80` | sanitized text length after copy-in |
| 6 | `sleep` | deadline delta `1..10000` ticks | `0` after the scheduler wakes the saved context |
| 7 | `send` | target PID `1..255`, fixed-width value | `0`, or a bounded error if the target is unavailable or its inbox is full |
| 8 | `receive` | all zero | next fixed-width inbox value; blocks if the inbox is empty |
| 9 | `wait_child` | child PID `1..255` | child exit status; blocks while an owned child remains live |
| 10 | `system_info` | writable address and exact structure size `72` | copies `UserSystemInfo` and returns `72` |
| 11 | `read_file` | path address/length and writable output address/capacity | ABI 5 compatibility read; blocks and returns a byte count |
| 12 | `open_file` | path address/length | blocks, then returns an opaque read-only handle or a bounded error |
| 13 | `read_handle` | handle and writable output address/capacity | blocks, copies from the kernel-owned offset, advances it, and returns a byte count |
| 14 | `stat_handle` | handle, writable address, and exact structure size `32` | copies `UserFileStat` and returns `32` |
| 15 | `close_handle` | handle | revokes the capability and returns `0`; stale or foreign values are rejected |
| 16 | `open_file_with_rights` | path address/length and rights mask | blocks, then returns an opaque handle with the granted read/write rights |
| 17 | `write_handle` | handle and mapped input address/length | copies at most 128 bytes into the kernel, blocks, writes at the owned offset, and returns a byte count |
| 18 | `wait_input` | writable event address, exact size `32`, and keyboard/pointer mask | blocks until a matching event is copied, then returns `32`; contention returns `USER_ERROR_UNAVAILABLE` |

The output path validates the whole range against the userspace window, translates every byte through the owning address space, rejects unmapped holes, and replaces control or non-ASCII bytes before the shell sees them. The application uses runtime functions instead of handwritten assembly. Cooperative yield remains available for ABI compatibility, but the execution proof relies on timer preemption.

Blocking syscalls copy the normalized interrupt frame into the process context and return to Ring 0. Sleeping and waiting processes are excluded from runnable selection. A wakeup writes the syscall result into saved `rax` before changing the state back to `Ready`, so execution continues immediately after the original `int 0x80` instruction.

Each managed process owns a four-value FIFO inbox. `send` either wakes a receiver directly or appends at the tail; it never allocates and never silently overwrites an older value. `wait_child` accepts only a live or retained process whose recorded parent PID matches the caller. These policies keep the first IPC contract small and deterministic.

`UserSystemInfo` is a `repr(C)` structure of nine `u64` fields: ABI version, page size, timer frequency, message capacity, maximum file-read size, file-handle capacity, maximum file-write size, input-event size, and supported input masks. `UserFileStat` has four `u64` fields: size, current offset, node kind, and rights. `UserInputEvent` contains `kind`, `code`, signed `value0`, and signed `value1`, each eight bytes. Their sizes, alignments, field offsets, rights, limits, errors, masks, and codes are tested. `UserProcessHeader` fixes the kernel-owned token and preemption words at offsets 0 and 8; those offsets are also tested so adding application data cannot silently break preemption again.

Input mask `1` selects keyboard events and mask `2` selects pointer events; callers may combine them. Keyboard events use kind `1`; printable characters place ASCII in `value0`, while Enter, Backspace, Escape, Tab, Arrow Up, and Arrow Down have stable codes and zero values. Pointer movement uses kind `2`, signed deltas in `value0`/`value1`, and the active button mask in `code`. Pointer button events use kind `3`, cursor position in the signed values, and left/right/middle bits `1`, `2`, and `4` in `code`.

Input waits are one-shot and unbuffered. At most one live process may own the wait channel. A second waiter is returned to `Ready` with `USER_ERROR_UNAVAILABLE`; it never displaces the owner. The desktop offers each queued hardware event to the owner first. A mask mismatch leaves the process blocked and the event continues through normal shell or window handling. A match copies the encoded event only after the stored destination is revalidated, clears ownership, injects the exact structure size into saved `rax`, and consumes that event so the shell cannot also interpret it.

Paths are 1–64 ASCII bytes, must be absolute, and may use only letters, numbers, `/`, `.`, `_`, or `-`. Read buffers are capped at 128 bytes and must remain inside the process's writable data page, with every byte translating to the physical frame owned by that process. A handle contains a process prefix, monotonically advancing per-process generation, and slot identity, but userspace must treat the value as opaque. Authority comes from an exact entry in the calling process's table; guessing another PID's value never grants access.

Legacy `open_file` grants read-only authority. `open_file_with_rights` accepts a nonzero mask containing `READ`, `WRITE`, or both; any unknown bit is rejected. A request containing `WRITE` is accepted only when the validated path starts with `/USER/` and contains a filename after that prefix. There are no traversal semantics, symlinks, or mount aliases in the RAM VFS, and the check is repeated at the kernel/VFS service boundary.

`stat_handle` exposes the open-time size and kind plus the live per-open offset and rights. Reads and writes resolve the kernel-owned path and offset rather than accepting either from userspace. A successful operation advances the offset by exactly the completed byte count; writes also extend the capability's observed size. Write payload bytes may come from any mapped readable user page but are copied into the pending kernel request before blocking. Completion must match the original task ID, Ring 3 PID, handle, path, offset, payload, and requested length. The older path-based `read_file` remains syscall 11 for compatibility, while new applications should use handles.

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
- File authority is represented by an exact process-owned handle entry with explicit rights and a per-open generation.
- Userspace cannot choose the path or offset of a handle read; both come from the kernel capability table.
- Successful reads advance the per-open offset by the exact copied byte count, while stat observes the same offset.
- Close and process termination revoke handles; stale reuse returns a stable invalid-argument error.
- Write-capable opens and writes are restricted to `/USER/`; protected paths never reach the mutation service.
- Write payloads are bounded to 128 bytes and copied into kernel-owned memory before blocking.
- Read-only handles reject writes, and successful writes advance both capability offset and observed size.
- Open, read, and write completions bind task ID, Ring 3 PID, handle generation, path, offset, limits, and payload identity.
- A blocked input waiter leaves runnable selection until one matching event is copied into its validated private data page.
- Keyboard and pointer masks are enforced before routing; a mismatch remains available to normal desktop handling.
- Input ownership is one-shot and exclusive, with explicit contention failure rather than event duplication or waiter replacement.
- Keyboard characters, special keys, pointer deltas, positions, and button bits have tested fixed-layout encodings.
- Exit, fault, or kill clears pending input ownership before the process address space is reclaimed.

## Current limitations

- `INIT.ELF` is the only packaged userspace program.
- The immutable initrd ELF is registered directly with the loader and is not copied into the small writable session VFS.
- Shell `wait PID` is observational; blocking semantics are available only to a userspace parent through `wait_child`.
- The process manager has four slots. A terminal process occupies one until `wait` reaps it.
- The recycled-frame pool is intentionally bounded to 256 frames; this milestone does not provide a general coalescing physical-memory allocator.
- Messages carry one `u64`; there are no byte streams, endpoint handles, permissions beyond process availability, or multi-producer fairness guarantees yet.
- The userspace file API is capped at four handles and 128 bytes per read or write, and it is backed only by the session RAM VFS. Directory capabilities, seek, truncation controls, shared handles, live metadata refresh, and persistent storage are not implemented yet.
- `/USER/` writes last only for the current boot; there is no durable block device or crash-consistency guarantee yet.
- The desktop holds one pending VFS request because it schedules at most one userspace slice and services one completion per tick.
- Input waits have no backlog, timeout, focus capability, per-window routing, or multi-waiter queue yet. Events that arrive before a wait remain desktop events.
- There is no heap allocator or userspace window API.
- Output is a bounded text syscall, not file-descriptor-based standard I/O.
- The transition state is single-core and supports one active user process at a time.

The next slice is multi-producer channel policy and endpoint handles, followed by moving the shell into userspace.
