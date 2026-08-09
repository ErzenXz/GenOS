# GenOS roadmap

This roadmap turns the GenOS vision into testable engineering milestones. Dates are intentionally omitted until the project has enough contributor velocity to forecast responsibly. A milestone is complete only when its acceptance criteria pass in automation or on documented hardware.

## Guiding rules

1. Keep `main` bootable.
2. Build vertical slices that produce observable behavior.
3. Stabilize contracts before growing ecosystems around them.
4. Measure claims about speed, memory, latency, and size.
5. Prefer one supported path over several unfinished paths.
6. Add hardware breadth only after the abstraction it depends on is proven.

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

## Stage 5 — Networking

**Status: complete**

Goal: establish a small, testable network stack before exposing broad application APIs.

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
- userspace socket API;
- network diagnostics application.

Delivered:

- GenOS 0.42 a QEMU NE2000 PIO driver with explicit free, driver, and stack packet-buffer ownership and no borrowed frame surviving a receive iteration;
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

## Stage 6 — Security and identity

Goal: make isolation and authority visible parts of the system architecture.

Planned work:

- user and service identities;
- capability or handle-based authority model;
- filesystem permissions;
- process sandbox profiles;
- entropy and random-number subsystem;
- signed package and update metadata;
- secure-boot research and measured-boot hooks;
- secrets storage design;
- security audit checklist and threat model.

Acceptance criteria:

- [ ] Applications receive only explicitly granted resources.
- [ ] A compromised unprivileged process cannot read another process's memory.
- [ ] Package and update signatures are verified before installation.
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

Candidate work:

- ACPI-based discovery and power control;
- SMP and multi-core scheduler support;
- APIC and modern interrupt routing;
- USB host controller and HID;
- NVMe;
- audio stack;
- higher-resolution and accelerated graphics;
- Wi-Fi research;
- laptop power and battery reporting;
- suspend and resume;
- installer and recovery environment.

Hardware support will be introduced through documented reference machines. “Works on my machine” is not an acceptance criterion; repeatable device reports and regression tests are.

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
