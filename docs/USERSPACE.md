# GenOS userspace boundary

GenOS 0.49 boots the interactive command shell as a separately linked Ring 3 process and gives it opaque console, VFS, namespace, process-lifecycle, and socket capabilities plus bounded network exchanges. ABI 17 keeps normal filesystem and process control in `SHELL.ELF` without granting authority through raw paths or PIDs, preserves generation-safe non-blocking UDP/TCP clients, and adds bounded TCP listener authority with one accepted request/response stream. This document states exactly what the milestone proves and what it does not.

## Build and packaging pipeline

1. `userspace/runtime` builds as a `no_std` library and owns the initial `int 0x80` syscall wrappers.
2. `userspace/init` and `userspace/shell` build as separate static `x86_64` executables using the dedicated `userspace` Cargo profile.
3. Userspace image layout 2 keeps `INIT.ELF` at one RX text page and gives `SHELL.ELF` a separate RW ABI data page plus up to eight RX text pages below the stack guard. Both layouts remain W^X.
4. `xtask` builds both executables before the kernel and packages them into the initrd as `INIT.ELF` and `SHELL.ELF`.
5. The kernel locates both images by name and excludes them from the writable session VFS. A missing or invalid required image stops boot safely.

The kernel binary no longer contains a `.usertext` payload. Userspace behavior comes from the ELF bytes supplied through the boot filesystem.

## ELF validation and mapping

The bounded parser accepts only little-endian ELF64 executable files for x86_64. It validates the ELF and program-header sizes, caps the program-header count, checks every offset and length with overflow-safe arithmetic, requires at least one loadable segment, and rejects truncated file data.

Before allocating user pages, the process loader additionally requires:

- page-aligned segment virtual addresses and at least page alignment;
- readable load segments with no unknown permission bits;
- write and execute permissions are never both present on one segment;
- segment memory ranges entirely inside the reserved user-image window;
- at most eight executable pages under image layout 2;
- no overlapping virtual pages;
- an entry point inside an executable segment;
- a writable data mapping at the ABI data address with space for the stable `UserProcessHeader` and application buffers.

Every accepted page receives a newly allocated zeroed physical frame. File bytes are copied into those frames, remaining memory is left zeroed, and page-table permissions come directly from the validated segment flags. Stacks are mapped separately with an unmapped guard page.

## Execution and lifecycle proof

At boot, GenOS creates three independent instances of `INIT.ELF` for the preemption and fault-containment proof:

1. all instances query ABI version 17 and become eligible for timer scheduling;
2. a 100 Hz PIT interrupt involuntarily preempts each process and saves its full CPU context;
3. the first instance writes to its guard page and is terminated with page-fault status 142 before performing output work;
4. the two healthy instances resume afterward, write greetings through the validated output syscall, report private values through validated copy-in, and exit with status 0.

GenOS then launches a fourth instance through the general ELF launch function and verifies preemption, output, private memory, normal exit, and reclamation. A separate lifecycle probe starts a normal asynchronous process and a persistent held process. The normal process outputs text, exits, and is reaped. The held process is preempted, killed with status 137, and reaped. Both release ten frames: two ELF pages, four stack pages, three user page-table pages, and one CR3 root.

The probe then creates an owned parent-child pair. Each has its own CR3 root. The parent publishes an endpoint and blocks while waiting on the exact child PID. The child blocks on a three-tick sleep deadline, wakes, connects to the parent's live PID, sends one value through the resulting send handle, closes that handle, and exits with status 7. Child termination injects that status into the parent's saved `rax` and returns the parent to `Ready`; the parent's subsequent receive copies out the queued message, checks that `sender_pid` is the child's PID, closes its endpoint, and exits with status 0. Both address spaces are reclaimed and both terminal records are reaped.

The fan-in probe then launches three processes: a receiver and its two producer children. Second-scale deadlines are intentionally wide enough to remain deterministic in both the headless smoke harness and the graphical desktop. Producer A sends `A1`; its immediate second send is refused with `USER_ERROR_UNAVAILABLE` while that message remains queued. Producer B independently sends `B1`. The receiver drains `A1` then `B1`, parks on its empty third receive, and is woken directly by producer A's later `A2`. The proof requires exactly three completed messages, one fairness denial, one direct wake, exact output, and reclamation of all three address spaces.

After the probes, GenOS launches `SHELL.ELF` persistently in a fresh address space and passes separate opaque console and lifecycle capabilities in its entry registers. Before announcing `USER_SHELL_READY`, the shell verifies image layout 2, directory enumeration, file mutation, parent-directory namespace authority, command-history recall, lifecycle control, and any available QEMU network. Its namespace proof creates and removes `/USER/ABI14`, rejects a forged parent handle, and confirms a repeated removal is unavailable. Its history proof inserts two commands, traverses backward and forward through the same Arrow Up/Down functions used by the input loop, and verifies the editor clears at the newest position. The shell owns an eight-command history and executes `help`, `echo`, `uname`, `net`, `clear`, `ls`, `cat`, `stat`, `touch`, `write`, `append`, `mkdir`, `rm`, `run init [hold]`, `ps`, `kill JOB`, and `wait JOB`.

Finally, a file-mode process requests `UserSystemInfo` through structured copy-out and opens `/README.TXT`. The open request blocks while the desktop VFS resolves a regular file; completion installs an opaque, read-only capability in the calling process's four-slot handle table. Ring 3 copies out `UserFileStat`, reads 17 bytes, confirms that `stat_handle` now reports offset 17, then reads the remaining 37 bytes through the same handle. The kernel derives the path and offset from the capability rather than trusting userspace. Each read blocks, and a scheduler poll confirms that no userspace slice runs while the request is outstanding.

The application compares all 54 bytes with the expected file, closes the handle, and proves that a subsequent read returns `USER_ERROR_INVALID_ARGUMENT`. The lifecycle probe also submits forged open and read completions before the valid completions and requires both to be rejected. Normal exit, fault, and kill revoke every handle still owned by the process.

A second file-mode process then requests read/write authority for `/README.TXT`; the kernel rejects it before issuing a VFS request because application mutation is limited to the `/USER/` tree. The process opens `/USER/APP.TXT` with read/write rights, causing the VFS service to create an empty file. It writes 13 and then 14 bytes through two blocking requests. Each payload is copied completely from mapped userspace into a fixed 128-byte kernel buffer before the process becomes non-runnable. Successful completion advances the capability offset and size to 13 and then 27.

After close, the application reopens the file read-only. A write through that narrower capability returns `USER_ERROR_INVALID_ARGUMENT` without reaching the VFS. The application reads back and verifies all 27 bytes, closes the handle, exits with status 0, and releases its ten address-space frames. The lifecycle probe submits a forged write offset before the real completion and requires rejection without changing authority or offset.

The probe next starts an input-mode process. After two bounded output calls it invokes `wait_input` with the keyboard mask; the process becomes `Waiting`, retains its writable event address, and cannot consume another scheduler slice. A synthetic pointer movement is presented first and deliberately does not wake the keyboard-only waiter. Pointer handling therefore remains available to the desktop rather than being stolen by an unrelated subscription.

While the first process still owns the one-shot input channel, a second input-mode process attempts the same wait. It receives `USER_ERROR_UNAVAILABLE`, reports that the channel is busy, exits normally, and releases its address space. The kernel then converts key `G` into the stable 32-byte `UserInputEvent`, revalidates the first process's private writable range, copies the event, writes `32` into saved `rax`, and returns that process to `Ready`. Ring 3 verifies the kind, code, printable value, and reserved field before reporting the exact key and exiting. Both task records are reaped and both ten-frame address spaces are reclaimed.

The QEMU smoke test requires markers for structured copy-out, file block/wake, exact content verification, directory copy-out, handle truncation, input block/filter/ownership/wake, sleep/block/wake, owned child wait/wake, message send/receive, endpoint capability, channel fairness, endpoint wake, fan-in ordering, frame reclamation, fault containment, and the long-lived desktop. Recycled roots are visibly reused by later processes in the serial proof.

## Desktop lifecycle

- `run init` asks the desktop coordinator to reserve a user task and construct a fresh process, then stores the returned opaque child handle under a monotonically increasing shell job ID.
- `run init hold` launches the same ELF with a persistent token. After its greeting, it remains runnable until controlled through that job handle.
- `run init sleep` blocks the process for three scheduler ticks and prints a second line only after its deadline wakeup.
- `run init file` copies out system metadata, opens `/README.TXT`, verifies stat and offset changes across two blocking reads, closes the handle, and proves stale reuse fails.
- `run init write` proves protected-path denial, creates `/USER/APP.TXT`, performs two bounded writes, checks stat, closes and reopens read-only, proves write denial, and verifies exact read-back.
- `run init input` waits for one printable keyboard event. Matching input is routed to the application instead of the shell, copied into private memory, reported, and consumed once.
- `run pair` reserves two task records and launches the parent-child coordination proof over one published endpoint. Task Manager exposes their `waiting`, `sleeping`, `ready`, and terminal transitions.
- `run fanin` reserves three task records and launches the receiver and two producers of the multi-producer proof: FIFO drain of `A1` and `B1`, one fairness denial, a blocking receive woken directly by `A2`, and `wait_child` reaping inside Ring 3.
- `ps` shows only the shell's owned jobs and copies status from the exact immutable process instance behind each handle.
- `kill JOB` terminates that owned live instance with status 137 and immediately releases its address space.
- `wait JOB` is non-blocking. It reports “still running” for a live instance; after exit, fault, or kill it copies terminal status, atomically consumes the handle, and frees the process-manager slot.

Completed task history remains in the task registry even after the heavier process resources have been reclaimed. When the bounded task table is full, a later launch may reuse a terminal history slot with a new PID.

The shell's `wait JOB` remains an observational reap command for operators. ABI `wait_child` is the blocking primitive used by a Ring 3 parent; the two operations intentionally serve different callers.

## ABI version 17

The syscall number is passed in `rax`. Scalar arguments use `rdi`, `rsi`, `rdx`, `r10`, `r8`, and `r9`. Results are returned in `rax`.

| Number | Runtime function | Arguments | Result |
| ---: | --- | --- | --- |
| 0 | `ping` | all zero | fixed GenOS reply value |
| 1 | `abi_version` | all zero | ABI version `17` |
| 2 | `exit` | status `0..255`; remaining arguments zero | terminates the current process instance |
| 3 | `yield_now` | all zero | cooperatively returns to the kernel scheduler |
| 4 | `report_u64` | owned user address and length `8` | validated value copied from user memory |
| 5 | `write` | mapped user address and length `1..80` | sanitized text length after copy-in |
| 6 | `sleep` | deadline delta `1..10000` ticks | `0` after the scheduler wakes the saved context |
| 7 | — | — | reserved: the legacy direct-PID `send`, removed in ABI 9 and never reassigned |
| 8 | — | — | reserved: the legacy inbox `receive`, removed in ABI 9 and never reassigned |
| 9 | `wait_child` | child PID `1..255` | child exit status; blocks while an owned child remains live |
| 10 | `system_info` | writable address and exact structure size `160` | copies `UserSystemInfo` and returns `160` |
| 11 | `read_file` | path address/length and writable output address/capacity | ABI 5 compatibility read; blocks and returns a byte count |
| 12 | `open_file` | path address/length | blocks, then returns an opaque read-only handle or a bounded error |
| 13 | `read_handle` | handle and writable output address/capacity | blocks, copies from the kernel-owned offset, advances it, and returns a byte count |
| 14 | `stat_handle` | handle, writable address, and exact structure size `32` | copies `UserFileStat` and returns `32` |
| 15 | `close_handle` | handle | revokes the capability and returns `0`; stale or foreign values are rejected |
| 16 | `open_file_with_rights` | path address/length and rights mask | blocks, then returns an opaque handle with granted read, write, or directory-management rights |
| 17 | `write_handle` | handle and mapped input address/length | copies at most 128 bytes into the kernel, blocks, writes at the owned offset, and returns a byte count |
| 18 | `wait_input` | writable event address, exact size `32`, and keyboard/pointer mask | blocks until a matching event is copied, then returns `32`; contention returns `USER_ERROR_UNAVAILABLE` |
| 19 | `create_endpoint` | all zero | publishes this process's endpoint and returns its opaque receive handle; `USER_ERROR_UNAVAILABLE` when one is already published or the four handle slots are full |
| 20 | `connect_endpoint` | target PID `1..255`; remaining arguments zero | returns an opaque send handle naming that PID and the generation it publishes now; `USER_ERROR_UNAVAILABLE` when the PID is not live, publishes nothing, or the caller's handle table is full |
| 21 | `send_endpoint` | nonzero send handle and a `u64` value; remaining arguments zero | `0` when the message is delivered or queued; `USER_ERROR_INVALID_ARGUMENT` for a handle that is not a live send capability or whose target generation is gone; `USER_ERROR_UNAVAILABLE` when this producer already has a message queued, the queue is full, or the copy into a parked receiver fails |
| 22 | `receive_endpoint` | own receive handle, writable address, exact size `16` | returns `16` after one `UserChannelMessage` is copied out; blocks while the queue is empty; `USER_ERROR_INVALID_ARGUMENT` for a handle that is not the currently published receive capability, a wrong length, or a buffer outside the process's writable data mapping |
| 23 | `close_endpoint` | handle; remaining arguments zero | `0`; closing a send handle revokes only that handle, closing the receive handle also drops the queue, unpublishes the endpoint, and revokes every remote send handle naming that generation. Unknown or stale values return `USER_ERROR_INVALID_ARGUMENT` |
| 24 | `console_write` | console handle, mapped text address, length `1..80`, line kind `0..3` | appends one sanitized output, prompt, error, or status line and returns its length |
| 25 | `console_set_input` | console handle, mapped text address, length `0..80` | replaces the editable terminal line; zero length clears it |
| 26 | `console_clear` | console handle; remaining arguments zero | clears terminal scrollback and returns `0` |
| 27 | `read_directory` | directory handle, ordinal cursor, writable entry address, exact size `96` | blocks; returns `96` for one direct child, `0` at end, or a bounded error |
| 28 | `truncate_handle` | write-capable regular-file handle; remaining arguments zero | blocks, sets file size and the handle offset to zero, and returns `0`; stale, read-only, directory, and protected-path handles are rejected |
| 29 | `process_launch` | exact shell supervisor capability, image `INIT`, mode normal or hold | blocks through the desktop coordinator and returns a new opaque process handle; altered authority, table exhaustion, or launch failure returns a bounded error |
| 30 | `process_status` | owned process handle, writable address, exact structure size `64` | copies status for the exact process incarnation and returns `64`; guessed, foreign, consumed, or stale handles are rejected |
| 31 | `process_kill` | owned process handle; remaining arguments zero | terminates a live target with status `137`; terminal targets return `USER_ERROR_UNAVAILABLE` without consuming the handle |
| 32 | `process_reap` | owned process handle, writable address, exact structure size `64` | for a terminal target, atomically copies status, consumes the handle, frees the manager slot, and returns `64`; a live target returns `USER_ERROR_UNAVAILABLE` |
| 33 | `create_directory` | managed parent-directory handle and child-name address/length | blocks, creates exactly one child directory beneath the owned parent, and returns `0` |
| 34 | `remove_path` | managed parent-directory handle and child-name address/length | blocks, removes one file or empty directory, revokes handles naming it, and returns `0` |
| 35 | `network_config` | writable address and exact structure size `24` | copies the configured IPv4 address, subnet, gateway, DNS, and MAC or returns unavailable |
| 36 | `udp_exchange` | IPv4 target, port, mapped request, writable response | compatibility one-shot UDP request/response with bounded buffers |
| 37 | `tcp_exchange` | IPv4 target, port, mapped request, writable response | compatibility one-shot active TCP request/response with bounded buffers |
| 38 | `socket_open` | UDP or TCP-stream protocol; remaining arguments zero | returns a generation-safe process-owned socket capability |
| 39 | `socket_connect` | socket handle, nonzero IPv4 target, port | records the remote; UDP becomes established and TCP enters connecting |
| 40 | `socket_send` | socket handle and mapped input `1..128` bytes | queues all bytes or returns `USER_ERROR_WOULD_BLOCK` without overwriting admitted data |
| 41 | `socket_receive` | socket handle and writable capacity `1..128` | copies available bytes while preserving any suffix, or returns `USER_ERROR_WOULD_BLOCK` |
| 42 | `socket_status` | socket handle, writable address, exact structure size `40` | copies protocol, state, readiness, and exact queued-byte counts |
| 43 | `socket_shutdown` | socket handle and read, write, or both direction mask | cancels queued work in the selected direction and records half/full-close state |
| 44 | `socket_close` | socket handle; remaining arguments zero | revokes and reclaims the object; stale or foreign values are rejected |
| 45 | `socket_bind` | TCP-stream handle and local port `1024..65535` | claims the port exclusively across live processes and enters `Bound`, or returns unavailable when another listener owns it |
| 46 | `socket_listen` | bound TCP-stream handle and backlog `1..2` | enters `Listening` with a fixed bounded pending-peer queue |
| 47 | `socket_accept` | listening TCP-stream handle; remaining arguments zero | returns a fresh established child capability for the oldest queued peer, or `USER_ERROR_WOULD_BLOCK` while the backlog is empty |

The output path validates the whole range against the userspace window, translates every byte through the owning address space, rejects unmapped holes, and replaces control or non-ASCII bytes before the shell sees them. The application uses runtime functions instead of handwritten assembly. Cooperative yield remains available for ABI compatibility, but the execution proof relies on timer preemption.

Blocking syscalls copy the normalized interrupt frame into the process context and return to Ring 0. Sleeping and waiting processes are excluded from runnable selection. A wakeup writes the syscall result into saved `rax` before changing the state back to `Ready`, so execution continues immediately after the original `int 0x80` instruction.

### Process lifecycle capabilities

Only the persistent shell receives a supervisor launch capability. It is tagged, generation-bound, compared exactly to the kernel-owned value, and cannot be reconstructed from a task ID or PID. A successful launch creates a fresh address space and an immutable kernel `ProcessKey` containing the manager slot and that slot's incarnation. The shell receives only an opaque handle stored in its own three-entry capability table; status, kill, and reap first resolve that local entry and then require the target slot to contain the same `ProcessKey`.

The shell assigns its own monotonically increasing display IDs, so `kill 2` means “use the handle recorded for shell job 2,” never “kill PID 2.” Reap is the sole consuming operation: terminal status is copied out before the handle and target slot are released. Killing or replacing a caller clears every pending lifecycle and VFS request. The desktop also rechecks a deferred VFS request against the still-parked process before touching the RAM filesystem, preventing a killed process's queued write or truncate from applying afterward.

The shell is also the session supervisor. Every process launched through its lifecycle capability records the shell's immutable `ProcessKey` as its owner. Shell exit, Ring 3 fault, and external kill all use the same terminal cleanup: owned children are killed with exit code `137`, immediately revoked and reclaimed, and removed instead of being transferred or retained as zombies. The shell's pending service work is canceled before it can mutate external state. Its terminal Task Manager row may remain until reap, but every child row is already gone and its cached console and lifecycle handle values are zeroed.

### Endpoint capabilities

Messaging authority is a capability, not a PID. A process may publish exactly one endpoint at a time with `create_endpoint`; the call returns a receive capability it owns. Every other process must obtain a separate send capability with `connect_endpoint` before it can send anything, and a raw PID by itself grants nothing.

Each process has a four-slot endpoint handle table shared by receive and send capabilities, so at most four endpoint capabilities can be held at once. Each published endpoint owns one four-message FIFO. Handles are opaque `u64` values that encode a dedicated endpoint tag byte, the owner PID, a monotonically advancing per-process generation, and the slot index. Resolution requires the tag, slot, owner PID, and generation to reproduce the handle exactly, so a guessed value, another process's handle, or a stale local handle never resolves. The endpoint tag occupies the byte that file handles use for their owner PID, so endpoint authority can never be spent on the file tables and file authority can never be spent on an endpoint.

Discovery is deliberately minimal: `connect_endpoint` takes a live PID and succeeds only while that process is not completed and currently publishes an endpoint. There is no name service, no directory, and no way to pass a handle to a third process. The returned send capability records the target PID together with the exact generation observed at connect time, which is what makes revocation total rather than best effort.

`send_endpoint` copies a `UserChannelMessage` whose `sender_pid` is filled in by the kernel from the calling Ring 3 process; a sender cannot forge another identity. Delivery is fair by admission: an endpoint queue holds at most one message per producer PID, so a second send from the same producer is refused with `USER_ERROR_UNAVAILABLE` while its first message is still queued, and is admitted again as soon as the receiver pops it. A single producer therefore cannot fill the four slots and starve its peers. Nothing is ever overwritten: a full queue and a duplicate producer both return errors rather than dropping an older message.

`receive_endpoint` pops the queue in arrival order. When the queue is empty the process leaves the runnable set with `BlockReason::Endpoint` after its output buffer has already been validated; the parked handle, generation, address, and length are stored in the kernel. A later send to that exact generation revalidates the stored destination, copies the sixteen bytes straight into it, writes `16` into saved `rax`, and returns the receiver to `Ready` without the message passing through the queue.

Revocation happens on every path that ends a capability. `close_endpoint` on a send handle frees that slot. `close_endpoint` on the receive handle unpublishes the endpoint, discards its queued messages, clears any parked receive, and sweeps every process's handle table to revoke send capabilities naming that generation. Normal exit, a Ring 3 fault, `kill`, and reap all run the same release path before the address space is reclaimed. A send through a handle whose target generation no longer exists is rejected with `USER_ERROR_INVALID_ARGUMENT` instead of reaching an unrelated process that later reuses the PID.

### Socket capabilities

Socket authority follows the same unified-table rule but has an independent `0xe7` tag, an encoded process slot/incarnation fragment, and an advancing per-process generation. A handle resolves only when its decoded slot, stored generation, exact value, typed `Socket` entry, and owning process slot/incarnation all agree. Closing and reopening the same metadata slot therefore produces a different value, another process cannot spend it, and PID or manager-slot reuse does not revive it.

ABI 17 owns four socket objects per process and two fixed 128-byte queues per object. UDP connect enters `Established`; TCP-stream connect enters `Connecting` until its first bounded client transaction completes or fails. A send is admitted atomically only when the whole payload fits. A receive may copy a prefix and retains the unread suffix. Empty reads and saturated writes return `USER_ERROR_WOULD_BLOCK`, which is distinct from invalid authority and unavailable resources.

`socket_status` exposes `Open`, `Connecting`, `Established`, `ReadClosed`, `WriteClosed`, `Closed`, `Failed`, `Bound`, or `Listening` plus readable, writable, connected, closed, error, and accept bits. Read/write shutdown clears the selected queued work before changing state. Full close removes both object metadata and typed authority. Exit, fault, kill, and supervisor cleanup close every socket owned by the exact process incarnation before its address space is reclaimed.

An admitted UDP datagram or TCP client request moves into a separate in-flight slot with a nonzero monotonic request ID. Completion requires the exact process slot, incarnation, task, PID, socket protocol, capability, request ID, and byte length to remain live. The runtime coordinator performs bounded ARP and transport progress outside the syscall, admits only a response from the exact address and port with valid checksums and protocol state, and copies at most 128 bytes into the owning receive queue. Completion moves a TCP socket from `Connecting` to `Established`. Timeout, RST, or bounded response overflow marks it `Failed` with error readiness. Write shutdown, close, exit, fault, or kill invalidates the request and prevents stale delivery.

ABI 17 binding is TCP-only and reserves ports below 1024. The process manager checks every live socket set before admitting a bind, so one local port has one owner system-wide. Listen fixes the backlog at one or two peers. Empty accept is non-blocking; an admitted peer makes the listener readable and accept-ready, and accept creates a separately revocable child capability in FIFO order. Allocation failure preserves the queued peer, handle-table failure rolls the child back, and close or owner cleanup makes the port immediately reusable.

The coordinator drives one checksum-validated passive SYN/SYN-ACK/ACK exchange for an exact live listener, then queues that peer only if the same process incarnation, PID, typed handle, port, and backlog authority remain valid. The accepted child retains the handshake's immutable MAC, address, ports, and initial sequences. It becomes readable only after exact wire validation and writable while its bounded send queue has capacity; it remains excluded from the active client engine. Accepted send completion uses its own nonzero request identity and requires the exact process slot, incarnation, task, PID, child handle, peer, byte count, and wire ACK.

The deterministic QEMU host sends `GENOS_PING`, half-closes, reads the Ring 3 `GENOS_PONG` response, and requires EOF after the guest FIN. Ring 3 waits for queued-send accounting to reach zero before write shutdown, then waits for wire FIN completion before closing the capability. The ordinary boot closes the optional listener after a bounded window when no peer exists. The shell also proves client socket authority, real UDP DNS, bounded TCP HTTP, RST/timeout/cancellation, listener denial and cleanup, and stale-handle rejection. Both active and passive paths remain one bounded request and response; concurrent clients, long-lived segmentation/reassembly, fair service, dynamic flow/congestion control, and production TCP remain Stage 5.4.

`wait_child` resolves the requested runtime PID only among children whose immutable parent process key matches the caller, then stores that exact child key while blocked. PID reuse cannot wake the wrong parent.

`UserSystemInfo` is a `repr(C)` structure of twenty `u64` fields and reports the four-socket limit, 128-byte queue budget, and 40-byte status layout. ABI 17 preserves image-layout version `2`, the eight-page executable capacity, and the separate 24-byte `UserNetworkConfig`. `UserSocketStatus` reports protocol, lifecycle state, readiness bits, and queued send/receive bytes. `UserProcessStatus` reports an immutable instance ID, task ID, informational runtime PID, state, exit code, fault vector, and preemption count. These identity fields are observations, never authority. `UserDirectoryEntry` remains 96 bytes, `UserChannelMessage` 16 bytes, `UserFileStat` 32 bytes, and `UserInputEvent` 32 bytes. Their sizes, alignments, offsets, rights, limits, errors, masks, and codes are tested.

Input mask `1` selects keyboard events and mask `2` selects pointer events; callers may combine them. Keyboard events use kind `1`; printable characters place ASCII in `value0`, while Enter, Backspace, Escape, Tab, Arrow Up, and Arrow Down have stable codes and zero values. Pointer movement uses kind `2`, signed deltas in `value0`/`value1`, and the active button mask in `code`. Pointer button events use kind `3`, cursor position in the signed values, and left/right/middle bits `1`, `2`, and `4` in `code`.

Input waits are one-shot. At most one live process may own the wait channel. A second waiter is returned to `Ready` with `USER_ERROR_UNAVAILABLE`; it never displaces the owner. The desktop offers each queued hardware event to the owner first. A mask mismatch leaves the process blocked and the event continues through normal window handling. While the designated console process is live but rearming, the desktop leaves queued input untouched. A matching event is copied only after its stored destination is revalidated, then the waiter wakes with the exact structure size in `rax`.

Paths are 1–64 ASCII bytes, must be absolute, and may use only letters, numbers, `/`, `.`, `_`, or `-`. Read buffers are capped at 128 bytes and must remain inside the process's writable data page, with every byte translating to the physical frame owned by that process. A handle contains a process prefix, monotonically advancing per-process generation, and slot identity, but userspace must treat the value as opaque. Authority comes from an exact entry in the calling process's table; guessing another PID's value never grants access.

Legacy `open_file` grants read-only authority. `open_file_with_rights` accepts `READ`, `WRITE`, and `MANAGE` rights; unknown bits and the unsafe `WRITE|MANAGE` combination are rejected. `WRITE` is restricted to regular files beneath `/USER/`. `MANAGE` requires `READ`, opens only an existing directory at or beneath `/USER`, and never creates a missing parent as a side effect. Namespace calls accept one validated child name, so separators, `.` and `..` cannot escape the owned directory.

`stat_handle` exposes the open-time size and kind plus the live per-open offset and rights. Reads and writes resolve the kernel-owned path and offset rather than accepting either from userspace. A successful operation advances the offset by exactly the completed byte count; writes also extend the capability's observed size. `truncate_handle` requires a live write-capable regular-file handle under `/USER/`; successful completion sets both VFS size and that capability's offset and observed size to zero. Write payload bytes may come from any mapped readable user page but are copied into the pending kernel request before blocking. Completion must match the original task ID, Ring 3 PID, handle, path, offset, payload, and requested length. The older path-based `read_file` remains syscall 11 for compatibility, while new applications should use handles.

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
- Child waits bind the caller and target by immutable process keys; a reused runtime PID cannot satisfy an older wait.
- Only `SHELL.ELF` receives launch authority, and every controlled child is represented by an opaque process-owned handle.
- Process status, kill, and reap resolve the exact slot incarnation; task IDs and runtime PIDs are informational only.
- Reap copies terminal status before consuming the handle and freeing the slot; live reap and stale reuse are rejected.
- Deferred VFS mutations are revalidated against the exact still-pending process request before the VFS changes.
- Messaging requires an endpoint capability: one published receive endpoint per process, and a separate process-owned send handle per connection.
- Endpoint handles are opaque, tagged, generation-checked, and resolvable only in the owning process's four-slot table.
- Every delivered message carries the kernel-supplied sender PID; a sender cannot forge another identity.
- Each endpoint queue is a four-message FIFO that admits at most one message per producer, so no producer can starve its peers.
- A full queue, a duplicate producer, and a stale target all return bounded errors instead of overwriting or dropping data.
- A blocking receive leaves runnable selection until a message is copied into its pre-validated buffer, either from the queue or directly from a sender.
- Closing a receive handle unpublishes the endpoint and revokes every remote send handle naming that generation; exit, fault, kill, and reap run the same release path.
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
- Truncation requires the same live write authority and resets the file and capability state together.
- Directory creation and removal require an exact process-owned parent handle carrying `MANAGE`; protected parents never reach the VFS.
- Removing a non-empty directory is rejected, and successful deletion revokes every open handle naming the removed path before it can be recreated.
- Open, read, and write completions bind task ID, Ring 3 PID, handle generation, path, offset, limits, and payload identity.
- A blocked input waiter leaves runnable selection until one matching event is copied into its validated private data page.
- Keyboard and pointer masks are enforced before routing; a mismatch remains available to normal desktop handling.
- Input ownership is one-shot and exclusive, with explicit contention failure rather than event duplication or waiter replacement.
- Keyboard characters, special keys, pointer deltas, positions, and button bits have tested fixed-layout encodings.
- Exit, fault, or kill clears pending input ownership before the process address space is reclaimed.
- `SHELL.ELF` is independently linked, mapped W^X, preempted, and kept as a persistent userspace task.
- Only the exact console handle granted at shell launch can append lines, replace the editor, or clear scrollback.
- Focused keyboard bursts remain queued until the Ring 3 shell rearms its one-shot input wait.
- `help`, `echo`, `uname`, `clear`, `ls`, `cat`, `stat`, `touch`, `write`, `append`, `mkdir`, `rm`, `run init [hold]`, `ps`, `kill JOB`, and `wait JOB` are parsed and executed outside Ring 0.
- Command history and Arrow Up/Down recall live in the shell's private userspace data page.

## Current limitations

- `INIT.ELF` and `SHELL.ELF` are the only packaged userspace programs. Their immutable initrd bytes are registered directly with the loader and not copied into the writable session VFS.
- Shell `wait JOB` is observational; blocking semantics are available only to a userspace parent through `wait_child`.
- The process manager has four slots and the shell can own three job handles. A terminal process occupies one until `wait JOB` reaps it.
- The recycled-frame pool is intentionally bounded to 256 frames; this milestone does not provide a general coalescing physical-memory allocator.
- A message carries one scalar `u64` payload plus the sender PID. There are no byte streams, buffers, or typed payloads.
- A process may publish only one endpoint at a time, and there is no name service: `connect_endpoint` needs a live PID.
- Handles cannot be delegated. A capability is usable only inside the process that created it, so there is no way to hand a send handle to a third process.
- The endpoint handle table holds four capabilities per process and each endpoint queue holds four messages, with at most one queued message per producer. A producer that wants to send again must wait for the receiver to drain its previous message.
- There are no priorities, timeouts, non-blocking receive, multicast, or reply correlation in the endpoint contract.
- The userspace file API is capped at four handles and 128 bytes per read or write. `/USER/` files and directories are synchronously committed through the partitioned `GFS2` mount, while `/TMP/` and root initrd files remain session-only. Arbitrary seek, nonzero-length resize, shared handles, and live metadata refresh across separate opens are not implemented yet.
- The durable mount inherits the RAM VFS limits of 32 nodes, 64-byte paths, and 512 bytes per file. Each mutation rewrites a complete alternating snapshot; there is no free-space allocator, large-file extent format, controller discovery, repair writer, or explicit read-only recovery boot yet.
- The runtime coordinator holds one pending VFS request because it schedules at most one userspace slice and services one completion per tick.
- Input waits have no general per-process backlog, timeout, per-window capability, or multi-waiter queue. Queue preservation while rearming is currently special to the designated console process.
- There is no heap allocator or userspace window API.
- Output is a bounded text syscall, not file-descriptor-based standard I/O.
- The transition state is single-core and supports one active user process at a time.

The duplicated normal command parser has been removed from the kernel. When `SHELL.ELF` is no longer live, the emergency parser accepts only `help`, `status`, `mem`, `reboot`, and `shutdown`; unit and boot checks reject file, namespace, lifecycle, and ordinary shell commands at that boundary.

Boot validation also emits two deliberately scoped scheduler measurements. `SCHED_DISPATCH_BENCH` runs the real round-robin policy for 64 ticks and reports dispatch count plus maximum and average ready-to-dispatch latency. `SCHED_CONTEXT_BENCH` measures 32 real kernel-to-process-to-kernel CR3 switch pairs with serialized TSC reads and reports minimum and average pair cycles. It excludes userspace execution and does not claim end-to-end response latency.

Stage 3 now routes scheduling, process polling, lifecycle launches, and VFS completion through `RuntimeCoordinator`. `ProcessManager` supplies every userspace row in the immutable Task Manager snapshot; `TaskRegistry` contains only system and worker scheduling records. The `HEADLESS_RUNTIME_READY` proof completes real shell VFS and child-launch requests before framebuffer construction, `PROCESS_SNAPSHOT_READY` verifies that every process slot and displayed userspace snapshot agree, `UNIFIED_HANDLE_TABLE_READY` audits exact typed authority, and `ASYNC_REQUEST_IDENTITY_READY` proves distinct VFS and lifecycle request identities. Separate boot probes reject wrong IDs, canceled writable work, and replayed completions.
