# GenOS roadmap

This roadmap turns the GenOS vision into testable engineering milestones. Dates are intentionally omitted until the project has enough contributor velocity to forecast responsibly. A milestone is complete only when its acceptance criteria pass in automation or on documented hardware.

## Guiding rules

1. Keep `main` bootable.
2. Build vertical slices that produce observable behavior.
3. Stabilize contracts before growing ecosystems around them.
4. Measure claims about speed, memory, latency, and size.
5. Prefer one supported path over several unfinished paths.
6. Add hardware breadth only after the abstraction it depends on is proven.

## Modernity and compatibility policy

GenOS may use a simple or historical device to bootstrap a subsystem, but legacy hardware is never the long-term architecture by accident.

- Once a modern implementation exists, normal builds and release tests use it by default.
- A legacy driver may remain only as an isolated, explicitly labelled fallback or compatibility test. It cannot satisfy a modern milestone's acceptance criteria.
- Protocol and application contracts must sit above device-specific boundaries so replacing hardware does not rewrite the whole subsystem.
- New virtual-device work targets current VirtIO 1.x interfaces with legacy transport disabled. New physical-storage work targets NVMe; new USB work targets xHCI; new interrupt work targets APIC/x2APIC and MSI/MSI-X; new graphics work targets a userspace compositor and modern display/GPU contracts.
- IPv4 remains supported, but production networking must be dual-stack IPv4/IPv6. Plaintext HTTP may be used only inside deterministic tests; credentials, packages, updates, and personal data require authenticated TLS 1.3 in userspace.
- GenOS will not invent cryptographic primitives. Security-sensitive protocols use reviewed implementations, test vectors, explicit trust policy, and negative-path tests.
- A milestone records the standard or hardware contract it implements, the intentionally unimplemented features, and the evidence proving the default path did not fall back.

## Stage 0 — Boot and kernel foundation

**Status: complete**

Delivered:

- repo-owned UEFI bootloader;
- kernel ELF loading;
- versioned boot information contract;
- initrd loading;
- framebuffer handoff;
- serial diagnostics;
- x86_64 GDT, TSS, IDT, and interrupt initialization;
- physical memory discovery and initial frame allocation;
- repeatable bootable-image generation.

Acceptance criteria:

- [x] QEMU reaches the kernel through the GenOS bootloader.
- [x] Invalid boot contracts halt safely.
- [x] The kernel reports readiness over serial.
- [x] CI can build the bootable image from a clean checkout.

## Stage 1 — Interactive desktop foundation

**Status: complete**

Delivered:

- backbuffered framebuffer renderer;
- dirty-region presentation;
- vector text rendering;
- PS/2 keyboard and mouse input;
- bounded input event queue;
- native cursor, windows, focus, dragging, closing, and taskbar;
- terminal with command history and common keyboard modifiers;
- writable session RAM filesystem;
- live Files and Task Manager applications;
- RTC-backed clock;
- long-running display and interrupt smoke markers.

Acceptance criteria:

- [x] The desktop stays responsive after the initial boot.
- [x] Keyboard and mouse input travel through the kernel event path.
- [x] Files reflects actual VFS state.
- [x] Task Manager reflects actual task-registry state.
- [x] Partial updates do not require presenting the full framebuffer.
- [x] The QEMU smoke test confirms interrupts continue after boot.

## Stage 2 — Processes and userspace

**Status: complete**

Goal: move from kernel-owned demo tasks to isolated executable processes.

Delivered so far:

- bounded process/task table and PID lifecycle;
- round-robin scheduler policy and quantum accounting;
- sleep/wake deadlines and protected system tasks;
- gap-safe physical frame allocation and a protected kernel address-space clone;
- initial privilege transition to ring 3;
- DPL3 syscall entry, scalar argument validation, return, and process exit;
- separate CR3 roots with private user code, data, guard, and stack mappings;
- saved general-purpose register and interrupt-return contexts;
- cooperative user yield, address-space switch, resume, and independent exit;
- bounded user-buffer validation and copy-in for the first pointer syscall.
- 100 Hz timer-driven userspace preemption without a cooperative syscall;
- process-local page-fault classification, termination, and fault status;
- continued execution of healthy processes after a peer faults.
- bounded ELF64 header and program-segment validation;
- page-aligned W^X mapping of independently built userspace executables;
- initial `no_std` userspace runtime with typed syscall wrappers;
- initrd packaging for `INIT.ELF` and boot-time executable discovery;
- asynchronous `run init` launch with fresh CR3, PID, task state, preemption, exit status, and shell output;
- persistent `run init hold` mode for observable live-process control;
- `kill PID` and non-blocking `wait PID` for userspace tasks;
- complete teardown of user leaf pages, page-table branches, and CR3 roots;
- bounded physical-frame recycling with double-free rejection and reuse tests;
- ABI 3 validated application output from mapped userspace memory;
- ABI 4 blocking sleep with scheduler tick deadlines and saved-context wakeup;
- explicit parent ownership and blocking wait on a specific child PID;
- bounded four-message per-process inboxes with blocking receive and direct wakeup, superseded by ABI 9 endpoints;
- `run init sleep` and `run pair` desktop proofs for coordination across isolated ELF instances;
- ABI 5 stable `UserProcessHeader` and typed `UserSystemInfo` copy-out contracts;
- mapped-range and physical-ownership validation before kernel-to-user copies;
- asynchronous `read_file` requests that leave Ring 3 blocked until the desktop VFS completes them;
- `run init file` proof of an exact 54-byte `/README.TXT` read and userspace verification.
- ABI 6 process-owned file handles with per-open generation values and read-only rights;
- blocking `open_file` and offset-aware `read_handle`, plus structured `stat_handle` copy-out;
- explicit `close_handle`, stale-handle rejection, and automatic handle revocation on termination;
- two-chunk `run init file` proof with offsets 0 and 17, exact content verification, and close misuse testing.
- ABI 7 explicit read/write capability rights and a shared 128-byte maximum write contract;
- protected `/USER/` mutation policy that keeps boot and system files read-only to applications;
- kernel-owned write payloads, blocking offset-aware VFS mutation, and stat size/offset updates;
- `run init write` proof covering two writes, protected-path denial, read-only denial, close/reopen, and exact read-back.
- ABI 8 fixed-layout keyboard and pointer events with stable masks, key codes, button bits, and signed values;
- one-shot `wait_input` copy-out that removes a blocked process from runnable selection;
- matching-event routing that leaves non-matching input available to the desktop;
- deterministic single-waiter ownership with explicit `USER_ERROR_UNAVAILABLE` contention behavior;
- `run init input` and boot proofs for wait, filter, contention, exact key wakeup, exit, and reclamation.
- ABI 9 endpoint capabilities replacing direct-PID messaging, with syscalls 19–23 and reserved-but-unassigned numbers 7 and 8;
- one published receive endpoint per process, process-owned send handles, and tagged generation-checked handles in a four-slot table;
- `connect_endpoint` discovery by live PID only, with no name service and no handle delegation;
- fixed-layout 16-byte `UserChannelMessage` carrying a kernel-supplied sender PID that Ring 3 cannot forge;
- four-message endpoint queues that admit at most one message per producer, so no producer starves its peers;
- blocking `receive_endpoint` with pre-validated buffers and direct sender-to-waiter copy-out that bypasses the queue;
- stale-handle rejection plus total revocation on close, exit, fault, kill, and reap;
- `run fanin` and the boot proof of a real three-process `A1`, `B1`, `A2` fan-in with one fairness denial and one direct wake.
- ABI 10 console capabilities with bounded line output, editable-input replacement, and clear operations;
- a separately linked and packaged `SHELL.ELF` with a private writable ABI data page;
- persistent Ring 3 shell launch with its own address space, task identity, preemption, and `USER_SHELL_READY` boot proof;
- focused keyboard delivery to the shell while compositor-owned `Escape` and `Tab` remain in Ring 0;
- queue-preserving input handoff while the one-shot shell waiter rearms, including burst input from QEMU;
- userspace parsing and execution of `help`, `echo`, `uname`, and `clear` through an opaque process-owned console capability.
- ABI 11 bounded directory-entry copy-out with cursor-based, direct-child enumeration;
- blocking directory requests resolved by the desktop VFS service without exposing kernel pointers;
- Ring 3 `ls` for `/` and named directories, plus handle-backed `cat` with bounded chunk reads;
- boot-time `USER_DIRECTORY_READ_OK` proof before `SHELL.ELF` announces readiness.
- ABI 12 process-owned `truncate_handle` with write-right and `/USER/` policy validation;
- Ring 3 `touch`, `write`, and `append` using blocking read/write file capabilities;
- boot-time create, truncate, write, reopen, and exact read-back proof from `SHELL.ELF`.
- ABI 13 shell-only launch authority plus three process-owned, opaque lifecycle handles;
- Ring 3 `run init [hold]`, `ps`, `kill JOB`, and `wait JOB` with monotonically increasing shell job IDs;
- exact process-instance status, kill, and consuming reap that reject guessed, foreign, live-reap, and stale handles;
- boot-time lifecycle proof covering launch, live status, kill status 137, reap, and post-reap rejection.
- ABI 14 versioned userspace image layout 2, expanding the shell RX budget from four to eight pages while retaining a guarded stack;
- an explicit directory-management capability right, plus parent-handle `create_directory` and `remove_path` operations;
- Ring 3 `stat`, `mkdir`, and `rm`, with non-empty-directory rejection and stale file-handle revocation after deletion;
- an eight-command history owned entirely by `SHELL.ELF`, including Arrow Up and Arrow Down recall.
- a standalone emergency Ring 0 parser limited to boot status, memory diagnostics, and power controls, with normal commands rejected by unit and boot checks;
- scheduler ready-to-dispatch latency accounting plus boot-time CR3 switch-pair cycle measurements.

Remaining work:

- none; further runtime ownership cleanup continues in Stage 3.

Acceptance criteria:

- [x] Kernel workers receive stable PIDs and reusable lifecycle slots.
- [x] Round-robin selection, CPU slices, and context-switch accounting are covered by tests.
- [x] Workers can sleep until a tick deadline, wake early, and terminate without affecting protected system tasks.
- [x] A boot-time program executes at ring 3 on explicitly exposed code and guarded stack pages.
- [x] The program crosses a DPL3 syscall gate, receives ABI results, and exits cleanly back to ring 0.
- [x] Initial syscall numbers and scalar arguments are validated before kernel dispatch.
- [x] Two independent userspace processes run with separate address spaces.
- [x] Both processes yield and resume with preserved CPU registers and private memory.
- [x] A validated user pointer is translated through the owning address space before copy-in.
- [x] A userspace crash terminates only the failing process.
- [x] The scheduler demonstrates preemption rather than cooperative polling.
- [x] Initial userspace pointer and buffer ranges are validated before kernel access.
- [x] A separately built ELF application is validated, mapped, preempted, and exited in isolated address spaces.
- [x] The shell can launch the packaged ELF and retain its completed task status.
- [x] The shell can asynchronously launch, inspect, terminate, and reap a userspace process.
- [x] Exit, fault, and kill reclaim every owned user image, stack, page-table, and root frame.
- [x] A userspace application can write bounded validated text to the desktop shell.
- [x] A sleeping userspace process leaves the runnable set and resumes at its deadline with preserved context.
- [x] A parent can block only on its own child and receive the child's exit status on wake.
- [x] Isolated processes can exchange fixed-width values through bounded kernel-owned message queues.
- [x] The kernel can copy a versioned structure into a validated process-owned writable mapping.
- [x] A userspace file read blocks without consuming slices and resumes with copied VFS bytes.
- [x] Cross-layer request identity and the kernel-owned process-header offsets are covered by checks.
- [x] A process can open a read-only file capability, advance a kernel-owned offset, inspect metadata, and close it.
- [x] Forged completions and stale handles are rejected without copying bytes or reviving authority.
- [x] A write-capable process can mutate only `/USER/`, with bounded kernel-owned payloads and offset accounting.
- [x] Protected paths and read-only handles reject writes, while successful data survives close/reopen inside the session VFS.
- [x] A userspace application can block on filtered keyboard or pointer input without polling or consuming slices.
- [x] Non-matching events remain available to the desktop, competing waiters fail explicitly, and one accepted event wakes exactly one process.
- [x] Messaging authority is a capability: a process must publish an endpoint to receive, and hold a process-owned send handle to send.
- [x] Handles are opaque, tagged, and generation-checked, so guessed, foreign, and stale values are rejected without granting access.
- [x] Every delivered message carries a kernel-supplied sender PID that Ring 3 cannot forge.
- [x] Two independent producers fan into one receiver, which drains them in arrival order with at most one queued message per producer.
- [x] A producer's second send is refused while its first message is still queued, and is admitted again after the receiver drains it.
- [x] A blocking receive leaves the runnable set and is woken directly by a later send that copies into its pre-validated buffer.
- [x] Close, exit, fault, kill, and reap revoke the endpoint and every remote send handle naming its generation.
- [x] A separately packaged shell runs persistently in Ring 3 and owns focused terminal command parsing.
- [x] Console mutation requires an exact process-owned capability; zero, guessed, and foreign handles are rejected.
- [x] Rapid keyboard input remains queued while the shell processes and rearms its one-shot input wait.
- [x] `help`, `echo`, `uname`, and `clear` execute in `SHELL.ELF` without the kernel command parser.
- [x] `ls` and `cat` execute in `SHELL.ELF` through blocking ABI 11 directory and file-capability calls.
- [x] Directory enumeration returns only direct children, has an explicit end cursor, and rejects missing or non-directory paths.
- [x] `touch`, `write`, and `append` mutate only `/USER/` files from `SHELL.ELF`; replacement truncates stale content before writing.
- [x] Filesystem and process-control commands execute in userspace through capability-backed APIs.
- [x] The recovery-only kernel command parser is removed.
- [x] Scheduler latency and context-switch cost are benchmarked.

## Stage 3 — Runtime cleanup and console-first system

**Status: complete**

Goal: strengthen the operating-system core before adding storage, networking, or a polished desktop. GenOS should first become a dependable console-first system with clean kernel/userspace boundaries. “Linux-like” here means a practical command-line userland, stable process and file abstractions, and composable tools—not Linux source, POSIX compatibility, or a copied Linux ABI.

Product direction:

- keep the serial terminal as the primary GenOS interface until the later graphical rebuild;
- freeze major visual redesign work during this stage;
- make system services independent of the old desktop loop and framebuffer availability;
- return to a real compositor, UI toolkit, and product-quality visual language in the later application and graphics stage.

Delivered so far:

- GenOS 0.23 `RuntimeCoordinator` ownership of the task registry, process manager, RAM VFS, scheduler advancement, lifecycle launch queue, and VFS completion queue;
- bounded runtime events and immutable task/VFS views for desktop presentation;
- removal of every lifecycle and VFS completion call from `kernel/src/shell.rs`, enforced by a host-side source-boundary test and the `RUNTIME_COORDINATOR_READY` QEMU marker;
- architecture notes covering current ownership, capability rights, request completion, cleanup, and the future window-server boundary.
- GenOS 0.24 removal of userspace lifecycle records from `TaskRegistry`; `ProcessManager` now composes every userspace Task Manager row into an immutable snapshot;
- boot-time display-disabled execution through real shell VFS and child-launch requests, required by `HEADLESS_RUNTIME_READY`, plus exact process/snapshot agreement required by `PROCESS_SNAPSHOT_READY`.
- GenOS 0.25 one typed, process-local handle table for file, endpoint, console, lifecycle, and process authority, with exact rights checks, cross-type rejection tests, coordinated revocation, and the required `UNIFIED_HANDLE_TABLE_READY` boot audit.
- GenOS 0.26 monotonic request IDs for every deferred VFS and lifecycle operation, bound to caller incarnation and operation type, with pre-mutation cancellation gates, one-shot completion, and the required `ASYNC_REQUEST_IDENTITY_READY` boot audit.
- GenOS 0.27 fail-closed shell supervision: exit, fault, and kill terminate and reap every owned child, cancel pending service work, revoke handles, reclaim address spaces, and remove child task rows through one terminal path, required by `SUPERVISOR_CLEANUP_READY`.
- GenOS 0.28 transactional runtime failure handling for full process and handle tables, refused launches, failed terminal copy-out, and canceled VFS service work, required by `RUNTIME_ROLLBACK_READY`.
- GenOS 0.29 a 257-launch real-address-space stress proof that crosses PID wrap, preserves monotonic process incarnations and handle generations, rejects stale authority, and returns every slot and frame, required by `PROCESS_GENERATION_STRESS_READY`.
- GenOS 0.30 a fresh-QEMU, display-independent transcript that types `echo qemu-console` and `uname` through the actual Ring 3 shell input and console syscalls, required by `CONSOLE_TRANSCRIPT_READY`.

Planned work:

- introduce a kernel runtime coordinator that owns scheduling, processes, tasks, and asynchronous service queues independently of `DisplayManager`;
- make process state the single source of truth and expose immutable snapshots to Task Manager instead of maintaining a second lifecycle;
- remove VFS and process-completion orchestration from the desktop shell loop;
- split the monolithic userspace kernel module into process, scheduler, syscall, capability, IPC, VFS-request, and boot-proof modules;
- evolve userspace image layout 2 only through explicit versioned contracts as executables grow;
- keep the explicit emergency recovery console separate from the normal Ring 3 command path;
- establish a small console userland with executable discovery, arguments, environment, standard input/output/error, exit status, and bounded composition between tools;
- document which interfaces intentionally resemble Unix and which remain native GenOS contracts.

Acceptance criteria:

- [x] Userspace scheduling, process lifecycle, and VFS completions continue working when desktop rendering is disabled.
- [x] `ProcessManager` or its replacement is the sole lifecycle authority; Task Manager reads snapshots and cannot drift from runtime state.
- [x] No VFS or process request is completed by `kernel/src/shell.rs` or `DisplayManager`.
- [x] Every handle resolves through one typed caller-owned table with explicit rights and tested cross-type rejection.
- [x] Every asynchronous completion is bound to an exact process incarnation and request ID, and canceled work cannot mutate external state.
- [x] Shell exit, fault, and kill terminate or transfer every owned child according to a documented policy and leave no stale task record.
- [x] Full tables, failed launches, failed copy-out, and service cancellation roll back without leaking tasks, slots, handles, or frames.
- [x] More than 256 sequential process launches complete without PID confusion or stale-handle authority.
- [x] Userspace executables have documented growth headroom or a loader contract that safely supports additional code segments.
- [x] A fresh QEMU test drives a real console command transcript without depending on mouse interaction or desktop timing.
- [x] The recovery-only kernel parser is removed from the normal command path.
- [x] Architecture notes define runtime ownership, handle rights, request lifecycle, process cleanup, and the later window-server boundary.

## Stage 4 — Persistent storage

**Status: complete**

Goal: preserve user data across boots without weakening filesystem correctness.

Planned work:

- PCI discovery needed by the first storage controller;
- choose and implement one initial virtualized block device;
- partition-table discovery;
- block cache and writeback policy;
- durable filesystem format;
- mount model integrated with the VFS;
- crash-consistency strategy;
- filesystem repair and inspection tool;
- read-only boot/recovery path.

Delivered so far:

- GenOS 0.31 a dedicated ATA-backed QEMU data image with a versioned, checksum-protected bounded record, host-side raw-image inspection, and a two-boot proof that restores `/USER/PERSIST.TXT`; the remaining VFS stays RAM-backed.
- GenOS 0.32 replaces the single record with two committed generation slots and restores the newest valid snapshot.
- GenOS 0.33 adds an independent `cargo xtask inspect-data` decoder plus checksum, bounds, duplicate-path, payload, and RAMFS-separation tests.
- GenOS 0.34 publishes healthy, recovered, or error state through read-only `/STORAGE.STATUS`; the Ring 3 shell proves both status reads and rejected write-capable opens.
- GenOS 0.35 recreates `/TMP/SESSION.TXT` in RAM on every boot and completes four QEMU phases: first commit, clean restore, torn newer-slot recovery, and corrupted-device failure.
- GenOS 0.36 adds an 8 MiB data disk with a bounds-checked MBR partition of type `0x7f`; the kernel discovers its start and length before any filesystem-sector access, and the host inspector rejects missing, malformed, or out-of-range partitions.
- GenOS 0.37 adds an eight-sector LRU write-back cache and mounts a two-generation `GFS2` snapshot containing general `/USER/` files and directories. Ring 3 file, truncate, directory-create, and remove operations synchronously commit before success is returned; `/USER/SHELL.TXT` is created by the shell, independently inspected on the host, and restored on the next QEMU boot.
- GenOS 0.38 makes the system server-first: the boot contract selects `console=serial ui=off`, the bootloader no longer requires GOP, QEMU runs without a display, and host-driven COM1 input reaches the Ring 3 shell.
- GenOS 0.39 discovers the PCI IDE controller, derives its compatibility or native I/O registers, enables PCI I/O space, and boots the persistent disk through the discovered controller.
- GenOS 0.40 adds `cargo xtask repair-data`, which reconstructs one damaged or blank snapshot only from the sole valid generation, verifies the resulting image, no-ops on healthy media, and refuses to invent data when neither generation is trustworthy.
- GenOS 0.41 adds partition type `0x7e` for explicit read-only recovery. The kernel restores the newest valid snapshot, advertises `state=readonly`, and rejects Ring 3 write and namespace-management authority while keeping files and `/TMP/` readable.

Acceptance criteria:

- [x] A file created in one QEMU session survives reboot.
- [x] Power interruption cannot silently corrupt unrelated files.
- [x] Filesystem images have host-side inspection tests.
- [x] Read/write failures are surfaced to applications.
- [x] The RAM filesystem remains available for temporary data.
- [x] The storage controller is discovered through PCI rather than assumed by the mount path.
- [x] A host repair writer restores snapshot redundancy without overwriting the only trustworthy generation.
- [x] Read-only recovery preserves readable files and denies persistent mutation end to end.

## Stage 5 — Network protocol foundation

**Status: complete**

Goal: establish a small, testable network stack before exposing broad application APIs. The original device was a bootstrap target, not the permanent virtual-hardware contract.

Planned work:

- one emulated network-device driver;
- packet-buffer ownership model;
- Ethernet framing;
- ARP;
- IPv4 and ICMP;
- UDP;
- DHCP;
- DNS resolver;
- TCP state machine;
- bounded userspace exchange API as a precursor to socket objects;
- network diagnostics application.

Delivered:

- GenOS 0.42 a bootstrap QEMU NE2000 PIO driver with explicit free, driver, and stack packet-buffer ownership and no borrowed frame surviving a receive iteration;
- bounded Ethernet II, ARP, IPv4, ICMP, UDP, and TCP parsing with IPv4 and transport checksum validation, fragment rejection, and truncation-safe host tests;
- DHCP discover/request/ack configuration with assigned address, subnet, gateway, and DNS state;
- ARP next-hop resolution and ICMP echo against the QEMU host gateway;
- ABI 15 `network_config`, `udp_exchange`, and `tcp_exchange` calls with validated user buffers and bounded request/response sizes;
- a Ring 3 DNS query for `example.com`, userspace answer parsing, and a TCP/HTTP request to a deterministic host test server;
- the Ring 3 `net` diagnostics command plus boot-visible DNS, HTTP, socket API, and timeout markers;
- three-attempt DHCP, ARP, UDP, TCP handshake, and initial-data retry policy with a bounded refused-connection proof.

Acceptance criteria:

- [x] GenOS obtains an address through DHCP.
- [x] ICMP echo works against the QEMU host network.
- [x] A userspace program resolves DNS and completes an HTTP request.
- [x] Malformed-packet tests do not panic or corrupt memory.
- [x] Packet loss and connection timeout behavior are defined.

## Stage 5.1 — Modern virtual network transport

**Status: complete**

Goal: make the supported virtual-machine network path standards-based, replaceable, and impossible to confuse with the legacy bootstrap driver.

Delivered:

- GenOS 0.43 a device-independent frame boundary used by the existing protocol stack;
- PCI discovery of VirtIO network functions and traversal of modern vendor capabilities;
- required `VIRTIO_F_VERSION_1` and MAC feature negotiation with the full status handshake and `FEATURES_OK` verification;
- independent eight-entry split RX and TX virtqueues with aligned descriptor, available, used, and DMA-buffer memory;
- the modern 12-byte `virtio_net_hdr`, bounded descriptor validation, explicit volatile DMA access, and acquire/release publication fences;
- QEMU `virtio-net-pci` with `disable-legacy=on` for normal and test boots;
- exact smoke markers that reject a false pass through any fallback path;
- NE2000 moved behind the device boundary and labelled as a legacy recovery fallback only;
- HTTP/1.1 with a required `Host` header for the deterministic Ring 3 test exchange.

Acceptance criteria:

- [x] Normal boot uses a VirtIO 1.x PCI network device with its legacy interface disabled.
- [x] DHCP, ICMP, DNS, TCP, and Ring 3 HTTP complete through VirtIO.
- [x] RX and TX descriptor ownership is bounded and observable.
- [x] The modern smoke suite fails if the kernel selects NE2000 or another fallback.
- [x] Boot remains healthy when no network device or deterministic HTTP server is present.

## Stage 5.2 — Non-blocking socket capability foundation

**Status: complete**

Goal: establish the process-owned object and queue contract before attaching applications to an asynchronous packet scheduler. This stage deliberately does not relabel the ABI 15 one-shot exchanges as sockets or claim production TCP.

Delivered:

- GenOS 0.44 and ABI 16 generation-safe, process-owned UDP and TCP-stream handles in the unified typed capability table;
- validated `socket_open`, `socket_connect`, `socket_send`, `socket_receive`, `socket_status`, `socket_shutdown`, and `socket_close` calls;
- explicit open, connecting, established, half-closed, closed, and failed states plus readable, writable, connected, closed, and error readiness bits;
- fixed four-handle and 128-byte-per-direction budgets, exact queued-byte accounting, partial receive preservation, `WOULD_BLOCK` backpressure, and no allocation on the syscall path;
- shutdown cancellation that clears the selected bounded queue, full close revocation, stale-generation rejection, forged-handle rejection, and process-exit owner cleanup;
- a Ring 3 lifecycle proof required by modern-network and normal-boot smoke tests, plus host tests for isolation, saturation, partial reads, failure visibility, cancellation, and reclamation;
- ABI 15 exchange compatibility retained only while the transport scheduler is built.

Acceptance criteria:

- [x] Socket handles are opaque, generation-safe, process-local capabilities and cannot be spent as another handle type.
- [x] Queue memory is statically bounded and saturation returns `WOULD_BLOCK` without overwriting admitted bytes.
- [x] Readiness, connecting state, partial receive behavior, half-close, close, stale handles, and owner cleanup have deterministic tests.
- [x] Ring 3 exercises the ABI 16 lifecycle on a modern-only VirtIO boot, including forged-handle denial and queued-work cancellation.
- [x] Documentation bounded this foundation separately from packet transport and kept production TCP as a release gate.

## Stage 5.3 — Asynchronous UDP socket transport

**Status: complete**

Goal: attach ABI 16 UDP queues to scheduler-driven packet progress without spinning inside the socket syscall, while preserving exact process and request authority across completion, timeout, and cancellation.

Delivered:

- GenOS 0.45 scheduler-owned UDP request extraction from process queues with a nonzero monotonic request ID bound to the exact process slot, incarnation, task, PID, socket handle, and in-flight datagram;
- one bounded coordinator transport slot that performs ARP resolution, UDP transmission, exact address/port/checksum demultiplexing, and receive-queue completion outside the syscall path;
- three-attempt deadlines, bounded per-tick NIC recovery polling, failed-socket readiness, queue-full refusal, shutdown cancellation, and stale completion rejection;
- a real Ring 3 DNS A query sent and received through the ABI 16 socket API on modern-only VirtIO, while the ABI 15 exchange remains as a compatibility proof;
- required QEMU markers for transport start, successful completion, timeout, and cancellation in both deterministic-server and normal no-server network boots;
- host tests that bind completion to the exact in-flight request and reject every truncation of an ARP reply.

Acceptance criteria:

- [x] A UDP socket queue makes wire progress without spinning in its syscall or blocking the kernel scheduler for an unbounded wait.
- [x] Only the exact live process incarnation, capability, request ID, and datagram can receive a completion.
- [x] DNS completes through ABI 16 sockets on modern-only VirtIO, and the smoke gate requires the exact asynchronous markers.
- [x] Timeout marks the socket failed and readable as an error; write shutdown cancels an in-flight request without later delivery.
- [x] Queue memory, retries, receive polling, response copies, and the coordinator transport slot are statically bounded.

Explicit boundary: this stage does not claim TCP socket transport, listeners, multi-socket fairness, interrupt-driven VirtIO completion, IPv6, or production Internet security. TX still uses bounded VirtIO completion polling, and RX uses bounded recovery polling once per coordinator tick until MSI-X work replaces it.

## Stage 5.4 — TCP socket transport, listeners, and production TCP

**Status: in progress; bounded clients plus one passive request/response stream complete; concurrency and production gates remain**

Goal: add server authority and make TCP correct under real loss, reordering, and flow-control pressure while replacing recovery polling with normal interrupt-driven packet completion.

### Stage 5.4A — Asynchronous TCP client transport

**Status: complete**

Delivered:

- GenOS 0.46 typed UDP/TCP in-flight requests using the same exact process-incarnation, capability, request-ID, payload, timeout, and cancellation authority checks;
- a bounded scheduler-driven TCP client transaction with ARP, SYN, exact SYN-ACK validation, ACK, request data, ordered response buffering, duplicate/out-of-order acknowledgment, FIN acknowledgment, active close, and RST failure;
- three-attempt deadlines for ARP, SYN, request retransmission, and final response progress, plus fixed 128-byte request/response storage and bounded NIC recovery polling;
- a real Ring 3 HTTP/1.1 request and 65-byte response through ABI 16 `socket_send` and `socket_receive`, while the no-server boot proves real RST failure without preventing boot;
- TCP in-flight write-shutdown cancellation with a protocol-specific stale-request marker and no later completion;
- mandatory modern-VirtIO QEMU markers for TCP transport start, completion, Ring 3 success, RST failure, and cancellation, plus host tests for protocol- and request-bound completion.

Acceptance criteria:

- [x] TCP connect, request, response, and close progress outside the socket syscall with bounded work per coordinator tick.
- [x] Only the exact live process, typed TCP capability, request ID, and admitted bytes can receive completion.
- [x] A deterministic HTTP response reaches Ring 3 through ABI 16 TCP sockets on modern-only VirtIO.
- [x] RST, timeout, response overflow, malformed tuples/checksums, and cancellation fail closed within fixed memory and retry budgets.

Explicit boundary: this client transaction supports one bounded request and response, not a general long-lived byte stream. It does not yet provide listeners, concurrent connections, fair cross-process scheduling, dynamic windows, congestion control, selective acknowledgments, full out-of-order reassembly, or interrupt-driven VirtIO completion.

### Stage 5.4B — TCP listener authority foundation

**Status: complete**

Delivered:

- GenOS 0.47 and ABI 17 `socket_bind`, `socket_listen`, and non-blocking `socket_accept` calls gated by the exact process-owned typed socket capability;
- TCP-only local ports from 1024 through 65535, with one exclusive owner across every live process and immediate release on close or process cleanup;
- `Bound` and `Listening` states, an explicit accept-readiness bit, and a fixed per-listener backlog of at most two pending peers;
- FIFO accepted-child creation as a fresh generation-safe TCP capability, with allocation failure preserving the pending peer and typed-handle registration failure rolling the child back;
- Ring 3 proof of low-port rejection, forged-handle rejection, empty non-blocking accept, oversized-backlog rejection, duplicate-bind refusal, stale-listener rejection, and close/rebind cleanup;
- host tests for global port exclusivity, bounded backlog refusal, FIFO accepted children, ownership isolation, readiness, capacity, and reclamation.

Acceptance criteria:

- [x] Bind, listen, and accept require an exact live TCP capability; UDP, forged, foreign, stale, low-port, and invalid-state requests fail closed.
- [x] A local port has one owner across live processes and becomes reusable after close or owner cleanup.
- [x] Listener and accepted-child memory is statically bounded, backlog saturation refuses new peers, and allocation failure does not consume queued work.
- [x] Every accepted child receives separately revocable typed authority and cannot be spent by another process.

Explicit boundary: the backlog is an internal bounded object-model contract only. The receive path does not yet perform passive TCP handshake processing or queue wire peers, so `socket_accept` returns `WOULD_BLOCK` in the current Ring 3 boot proof. This stage does not claim a working inbound server, concurrent host clients, fair listener wakeups, or long-lived accepted streams.

### Stage 5.4C — Bounded passive handshake and Ring 3 accept

**Status: complete**

Delivered:

- GenOS 0.48 scheduler-driven passive TCP receive processing for one exact live listener, with checksum, IPv4 destination, port, peer tuple, flags, and sequence validation;
- bounded SYN-ACK retransmission and timeout, duplicate-SYN handling, RST failure, explicit refusal for missing listeners or saturated backlogs, and cancellation when listener authority disappears;
- final-ACK admission through the exact process slot, incarnation, PID, typed listener handle, local port, and fixed backlog before Ring 3 can accept a fresh child capability;
- a deterministic QEMU host-forwarded connection to guest port 18081 requiring `TCP_PASSIVE_SYN_ACCEPTED`, `TCP_PASSIVE_HANDSHAKE_OK`, and `USER_SOCKET_PASSIVE_ACCEPT_READY`;
- a separate no-host QEMU boot proving the optional listener window does not stall normal startup;
- host tests for truncated passive SYN frames, TCP checksum corruption, exact SYN/final-ACK classification, and the rule that accepted children cannot accidentally enter the outbound client transport.

Acceptance criteria:

- [x] A real host SYN reaches only the listener owning its destination port and completes a bounded SYN/SYN-ACK/ACK exchange.
- [x] The established peer enters only that listener's backlog and becomes a separately revocable Ring 3 capability through non-blocking accept.
- [x] Missing, stale, closed, or saturated listeners fail closed without granting a child or keeping an unbounded handshake alive.
- [x] The deterministic passive proof and ordinary no-host boot both pass on modern-only VirtIO 1.x.

Explicit boundary at completion: Stage 5.4C proved one passive handshake and capability admission, not a TCP server data path. Accepted children exposed connected identity but remained deliberately non-writable; Stage 5.4D supplies the first bounded payload/close transaction while multiple simultaneous handshakes, fair listener wakeups, and long-lived service remain open.

### Stage 5.4D — Bounded accepted-stream transaction and close

**Status: complete**

Delivered:

- GenOS 0.49 carries the exact peer MAC, IPv4 address, remote/local ports, and initial sequence state from the completed handshake through backlog admission into the accepted child;
- one fixed 128-byte passive receive buffer and one fixed 128-byte send buffer, with exact tuple, checksum, sequence, and acknowledgment validation before queue mutation;
- Ring 3 receive through the accepted capability, response admission under a nonzero request identity bound to the exact process slot, incarnation, task, PID, handle, peer, and byte count, plus ACK-gated completion;
- bounded response and FIN retransmission, duplicate/out-of-order acknowledgment, oversized-segment reset/failure, peer RST failure, peer half-close, guest FIN, and wire-ACK completion before the socket enters `Closed`;
- stale listener, accepted capability, or send identity cancellation with an on-wire reset and no later delivery;
- a QEMU host-forwarded `GENOS_PING` → `GENOS_PONG` exchange that verifies the response and EOF, while the no-host boot still completes without passive success markers.

Acceptance criteria:

- [x] Ring 3 receives bounded peer bytes only through the exact accepted child and can queue a bounded response through that same capability.
- [x] Send completion requires the exact peer ACK and the socket does not report `Closed` until the guest FIN is acknowledged on the wire.
- [x] Peer half-close, reset, stale capability, timeout, malformed acknowledgment, and oversized payload paths fail closed within fixed memory and retry budgets.
- [x] Deterministic modern-VirtIO QEMU proves request, response, peer FIN, guest FIN, and normal no-host degradation.

Explicit boundary: Stage 5.4D is one bounded request and response on one passive stream. It does not provide simultaneous handshakes, multiple accepted clients, arbitrary stream-length segmentation/reassembly, fair listener wakeups, dynamic windows, congestion control, production loss recovery, or an application server framework.

### Remaining Stage 5.4 work

- simultaneous passive handshakes, concurrent accepted-client lifecycle, long-lived segmented streams, and fair listener service;
- scheduler wakeups, readiness waits, request cancellation, fair queue service, and cross-process resource budgets;
- retransmission timers, RTT estimation, dynamic send/receive windows, congestion control, out-of-order reassembly, duplicate handling, reset handling, and TCP half-close on the wire;
- deterministic packet loss, duplication, delay, reordering, zero-window, reset, slow-reader, cancellation, and resource-exhaustion tests;
- interrupt-driven VirtIO completion, MSI-X, queue recovery, and measured batching before optional offload negotiation.

Acceptance criteria:

- [x] TCP socket queues make progress without spinning in a syscall or blocking the kernel scheduler for an unbounded wait.
- [x] One bounded passive handshake feeds the exact listener backlog and produces a Ring 3 accepted capability.
- [x] One accepted capability completes a bounded request, response, peer half-close, guest FIN, and wire-acknowledged close.
- [ ] A listening service survives concurrent clients, cancellation, peer resets, and slow readers without sharing raw port authority.
- [ ] TCP transfers remain correct under deterministic loss and reordering and respect bounded memory budgets.
- [ ] VirtIO RX/TX normally completes through interrupts; polling remains only a bounded recovery mechanism.
- [ ] Throughput, latency, packet-loss recovery, CPU cost, and queue occupancy have reproducible regression budgets.

## Stage 5.5 — IPv6 dual stack

**Status: planned; required before production networking**

Goal: make IPv6 a first-class path while preserving explicitly tested IPv4 compatibility.

Planned work:

- IPv6 packet validation, extension-header policy, routing, and path-MTU handling;
- ICMPv6, neighbor discovery, duplicate-address detection, router advertisements, and SLAAC;
- DNS AAAA resolution and address-selection policy;
- IPv6-capable UDP and TCP sockets with dual-stack bind/connect semantics;
- deterministic IPv4-only, IPv6-only, and dual-stack QEMU networks.

Acceptance criteria:

- [ ] GenOS configures a usable IPv6 address and default route without a hard-coded guest address.
- [ ] DNS and TCP complete on IPv6-only and dual-stack reference networks.
- [ ] Malformed extension headers, neighbor advertisements, and ICMPv6 packets fail closed.
- [ ] IPv4 and IPv6 behavior share socket contracts without hiding address-family differences.

## Stage 6 — Security and identity

Goal: make isolation and authority visible parts of the system architecture.

Planned work:

- user and service identities;
- capability or handle-based authority model;
- filesystem permissions;
- process sandbox profiles;
- entropy and random-number subsystem;
- isolated userspace TLS 1.3 with reviewed cryptography, certificate-path validation, hostname verification, a versioned trust store, and secure time policy;
- HTTPS for packages, updates, credentials, tokens, and personal data, with plaintext network authority denied to those applications;
- signed package and update metadata;
- secure-boot research and measured-boot hooks;
- secrets storage design;
- security audit checklist and threat model.

Acceptance criteria:

- [ ] Applications receive only explicitly granted resources.
- [ ] A compromised unprivileged process cannot read another process's memory.
- [ ] Package and update signatures are verified before installation.
- [ ] TLS 1.3 interoperability and negative certificate tests pass without custom cryptographic primitives.
- [ ] No credential, package, update, or personal-data flow can silently downgrade to plaintext.
- [ ] The project publishes a threat model for each trusted boundary.
- [ ] Security-sensitive unsafe code has dedicated review coverage.

## Stage 7 — Application and graphics platform

Goal: make native GenOS software practical to build, distribute, and run.

Planned work:

- stable application ABI or versioned compatibility contract;
- window-server/compositor boundary;
- shared-memory graphics surfaces;
- structured UI toolkit;
- text shaping and scalable fonts;
- clipboard and drag-and-drop contracts;
- application manifest and package format;
- SDK, templates, and documentation;
- package repository design;
- accessibility primitives;
- application lifecycle and background-execution policy.

Acceptance criteria:

- [ ] An application can be built outside the main repository using the SDK.
- [ ] Old applications receive a clear compatibility guarantee or failure mode.
- [ ] Applications cannot draw into another application's surface.
- [ ] Keyboard-only navigation works across reference system applications.
- [ ] Package installation is transactional and verifiable.

## Stage 8 — Hardware and daily-use expansion

Goal: grow beyond a virtual-machine reference platform without losing reliability.

Required modern baseline:

- ACPI-based discovery and power control;
- SMP and multi-core scheduler support;
- local APIC/x2APIC, I/O APIC, and MSI/MSI-X interrupt routing, with the 8259 PIC retained only for bootstrap fallback;
- xHCI USB host controller and USB HID, with PS/2 retained only as a labelled compatibility fallback;
- NVMe as the primary physical-storage contract, with ATA PIO retained only for recovery and simple-emulator coverage;
- IOMMU research and explicit DMA isolation policy before untrusted-device support;
- PCIe capability, hotplug, power-state, and error-reporting policy;

Expansion after the baseline:

- audio stack;
- virtio-gpu for the reference VM and a documented modern physical-GPU strategy behind the Stage 7 userspace compositor;
- Wi-Fi research;
- laptop power and battery reporting;
- suspend and resume;
- installer and recovery environment.

Legacy-only boot is not sufficient to complete this stage. Hardware support will be introduced through documented reference machines with device reports, fault injection, suspend/resume cycles, and regression tests. “Works on my machine” is not an acceptance criterion.

Acceptance criteria:

- [ ] The reference VM boots with modern VirtIO network, block, console, and GPU interfaces and rejects unintended legacy fallback in release tests.
- [ ] A documented x86_64 reference machine boots from NVMe, routes interrupts through APIC/MSI-X, and uses xHCI for input and removable media.
- [ ] DMA-capable drivers validate descriptor ownership, lengths, device reset, timeout, and surprise-removal paths.
- [ ] Suspend/resume, power loss, hotplug, and device-failure tests preserve filesystem and process isolation guarantees.
- [ ] Legacy compatibility drivers are build-time or boot-policy choices with explicit diagnostics, never silent defaults.

## Stage 9 — Graphical experience rebuild

**Status: deferred; the serial terminal is the primary interface until this stage**

Goal: replace the current experimental desktop with a deliberately designed graphical system after the kernel, storage, networking, security, application, and hardware contracts are stable. The existing framebuffer desktop is not a quality target and is not required for server-mode readiness.

Planned work:

- remove or replace the current desktop composition and visual language;
- define a cohesive interaction model, information architecture, and design system before implementation;
- build the graphical shell on the Stage 7 userspace window-server boundary instead of placing product UI in Ring 0;
- create a real terminal application that preserves every server-console workflow;
- add scalable typography, layout, theming, accessibility, keyboard navigation, and input-method contracts;
- provide reference Files, Tasks, Settings, recovery, and application-launch surfaces;
- establish screenshot, interaction, accessibility, and performance regression suites;
- validate the complete experience at multiple resolutions and on reference physical hardware.

Acceptance criteria:

- [ ] The framebuffer desktop contains no product UI implemented directly in the kernel.
- [ ] The graphical shell and reference applications run as isolated userspace processes.
- [ ] Every administrative workflow remains possible from the serial terminal without graphics.
- [ ] Keyboard-only and screen-reader-oriented navigation pass documented reference flows.
- [ ] Visual regression tests cover supported resolutions, scaling factors, focus states, errors, and recovery.
- [ ] Real-device testing proves input, rendering, launch, suspend, and recovery behavior.

## Cross-cutting tracks

These tracks continue throughout every stage.

### Reliability

- deterministic host-side tests where hardware is not required;
- QEMU smoke coverage for every boot-critical subsystem;
- fault injection for allocation, I/O, and malformed-input paths;
- panic diagnostics that remain useful without the desktop.

### Performance

- publish boot-time, memory, binary-size, input-latency, and idle-work baselines;
- define regression budgets before optimizing benchmarks;
- document benchmark hardware and QEMU configuration;
- avoid performance claims without reproducible evidence.

### Developer experience

- keep `make build`, `make run`, and `make test` reliable;
- provide architecture decision records for major contracts;
- label approachable issues with realistic scope;
- keep contributor setup documented for macOS and Linux hosts.

### Documentation

- maintain a boot-flow diagram;
- document unsafe invariants next to their implementation;
- add subsystem design notes before interfaces become public contracts;
- keep current limitations visible in the main README.

## How roadmap changes are made

Roadmap changes should be proposed through an issue or pull request that explains:

1. the user or developer problem;
2. why the work belongs in the current stage;
3. the smallest useful vertical slice;
4. acceptance criteria;
5. new security, compatibility, and maintenance costs.

The roadmap is a planning tool, not a promise to merge every listed idea. Working code, clear contracts, and long-term maintainability decide priority.
