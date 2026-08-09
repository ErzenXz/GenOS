# Runtime ownership

GenOS 0.42 separates operating-system runtime work from presentation, removes duplicated userspace lifecycle state, gives every process one typed handle authority table, binds every deferred service operation to an exact request identity, and makes the shell a fail-closed session supervisor. `RuntimeCoordinator` owns the kernel task registry, `ProcessManager`, mounted VFS, persistent snapshot coordinator, scheduler advancement, and the pending lifecycle and VFS queues. The active product surface is the serial terminal; the old `DisplayManager` code is deferred until the later graphical rebuild.

## Ownership map

| State or operation | Current owner | Consumer |
| --- | --- | --- |
| process slots, incarnations, saved contexts, capabilities | `ProcessManager` | `RuntimeCoordinator` |
| system and worker scheduler accounting | `RuntimeCoordinator` through `TaskRegistry` | composed task snapshot |
| userspace lifecycle and displayed process state | `ProcessManager` | composed task snapshot |
| mounted filesystem and VFS mutations | `RuntimeCoordinator` through `RamVfs` and `PersistentFs` | Ring 3 file capabilities |
| pending lifecycle and VFS requests | `RuntimeCoordinator` | no direct presentation access |
| interactive input | COM1 serial driver | runtime for Ring 3 delivery |
| console lines and editable input presentation | serial terminal loop | COM1 |
| graphical presentation | deferred | Stage 9 graphical rebuild |

`TaskRegistry` no longer stores userspace records. It contains only system tasks and kernel workers. On each coordinator iteration, those records are copied into a bounded `TaskSnapshotSet`, then `ProcessManager` appends one row per occupied process slot. Task Manager receives only that immutable set. Boot validation compares its user-row count, task IDs, names, classes, and states against the active process slots before emitting `PROCESS_SNAPSHOT_READY`.

## Request lifecycle

Ring 3 syscalls create kernel-owned request values containing a nonzero monotonic request ID plus the caller task, PID, process slot, and process incarnation. The enclosing request variant is the operation identity. File operations additionally carry the exact capability handle, resolved path, offset, rights, capacity, and copied payload needed by that operation. VFS and lifecycle operations share one request sequence within each process incarnation, so no two outstanding or completed operations from that incarnation reuse an identity.

`ProcessManager::poll` may emit one request after a userspace slice. The coordinator stores it in the matching bounded queue and validates the request ID, operation, process incarnation, task, PID, parked parameters, and waiting state before any filesystem mutation or process allocation. Exit, fault, and kill clear the parked record, which makes the matching coordinator request inactive. Stale and canceled requests are dropped before service. A successful completion consumes its parked record, so replaying the same completion is rejected. The resulting `ProcessUpdate` is copied into a bounded runtime batch for presentation.

The current coordinator has one pending VFS request and one pending lifecycle request because the scheduler runs at most one userspace slice per tick. During headless boot, `run_headless_boot_probe` advances the actual Ring 3 shell until VFS and child-lifecycle requests with distinct IDs have completed. The boot suite also kills a process with a writable open pending, proves the request becomes inactive without creating its target, and rejects a replay after a successful completion. `ASYNC_REQUEST_IDENTITY_READY`, `USER_ASYNC_CANCELLATION_OK`, and `USER_ASYNC_ONE_SHOT_OK` are required smoke markers. Multi-request queues remain future runtime work.

## Handle rights

Console, file, endpoint-send, endpoint-receive, lifecycle, and process handles all register in one bounded `HandleTable` owned by the caller. The table records the exact opaque ABI value, type, and rights. File operations request `READ`, `WRITE`, or `MANAGE` at lookup; the other families request their typed use/control right. Subsystem arrays hold only object metadata and cannot grant authority on their own.

Allocation succeeds only when both the typed table and matching metadata slot can be populated. Close, endpoint unpublish, namespace deletion, exit, fault, kill, and reap remove typed authority with their metadata. Unit tests prove wrong-type and insufficient-right rejection plus caller-table isolation. During headless boot, the runtime audits every live process, checks that each metadata object has exactly one matching typed entry, checks the expected cross-type failures, and emits `UNIFIED_HANDLE_TABLE_READY`.

## Process cleanup

Exit, fault, controlled kill, and external task kill enter the same `ProcessManager` terminal path. Cleanup revokes file, endpoint, console, lifecycle, and process handles; clears pending input and service requests; then reclaims user pages, page-table branches, and the CR3 root. Immutable process role is stored separately from revocable authority, so a terminal shell remains identifiable in its final snapshot without retaining a usable console or supervisor handle.

The shell owns every process launched through its supervisor capability. The policy is deliberately fail-closed: when the shell exits normally, faults, or is killed, each direct owned child is forced to exit code `137`, has all resources revoked, has its address space reclaimed, and is removed immediately rather than transferred or left as a zombie. Any pending shell lifecycle or VFS request becomes inactive before service. The shell itself remains as one terminal snapshot until its normal reap step; no child task row remains. Boot validation repeats this with real shell and child address spaces for all three terminal causes and requires `SUPERVISOR_CLEANUP_READY` plus the no-stale-task, no-stale-handle, and pending-cancellation markers.

The closing Stage 3 probes fill every process and child-handle slot, refuse another launch, exercise a launch without a task allocation, reject a failed terminal-status copy-out without consuming the child or its authority, and cancel a parked VFS request by killing its owner. Every case returns to the exact baseline process, slot, handle, and frame counts before `RUNTIME_ROLLBACK_READY`. A separate real-address-space loop launches, kills, copies terminal status, and reaps 257 children. PIDs necessarily wrap and are observed being reused, while monotonically increasing slot incarnations and process-handle generations reject the first stale handle throughout. `PROCESS_GENERATION_STRESS_READY` is emitted only after the owner and all children are reclaimed.

During headless boot, the runtime also waits for the Ring 3 shell's input subscription, injects `echo qemu-console` and `uname` as individual input events, and observes the exact prompt and output console writes. A separate QEMU phase attaches host stdin/stdout to COM1, sends `uname`, requires `SERIAL_RX_OK`, and observes the Ring 3 response. Neither proof invokes a kernel command parser. The smoke suite requires `CONSOLE_TRANSCRIPT_READY` and a successful host serial transcript.

## Window-server boundary

The old framebuffer desktop implementation remains in the tree as deferred development code, but the active boot path does not initialize or enter it. Any future presentation layer must remain downstream of the runtime: it must not poll processes, advance scheduling, reserve lifecycle slots, mutate the VFS on behalf of Ring 3, or complete asynchronous requests.

A later application stage can replace `DisplayManager` with a window server without changing process, capability, or VFS service ownership. The display-disabled boot proof now covers scheduling, VFS and lifecycle completion, exact asynchronous identities, cancellation, snapshots, and unified handle authority without coupling those services back to rendering.
