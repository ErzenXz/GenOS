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

**Status: in progress**

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
- bounded four-message per-process inboxes with blocking receive and direct wakeup;
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

Remaining work:

- multi-producer channel policy and endpoint handles;
- move the shell into userspace.

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
- [x] Isolated processes can exchange fixed-width values through bounded per-process inboxes.
- [x] The kernel can copy a versioned structure into a validated process-owned writable mapping.
- [x] A userspace file read blocks without consuming slices and resumes with copied VFS bytes.
- [x] Cross-layer request identity and the kernel-owned process-header offsets are covered by checks.
- [x] A process can open a read-only file capability, advance a kernel-owned offset, inspect metadata, and close it.
- [x] Forged completions and stale handles are rejected without copying bytes or reviving authority.
- [x] A write-capable process can mutate only `/USER/`, with bounded kernel-owned payloads and offset accounting.
- [x] Protected paths and read-only handles reject writes, while successful data survives close/reopen inside the session VFS.
- [x] A userspace application can block on filtered keyboard or pointer input without polling or consuming slices.
- [x] Non-matching events remain available to the desktop, competing waiters fail explicitly, and one accepted event wakes exactly one process.
- [ ] Scheduler latency and context-switch cost are benchmarked.

## Stage 3 — Persistent storage

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

Acceptance criteria:

- [ ] A file created in one QEMU session survives reboot.
- [ ] Power interruption cannot silently corrupt unrelated files.
- [ ] Filesystem images have host-side inspection tests.
- [ ] Read/write failures are surfaced to applications.
- [ ] The RAM filesystem remains available for temporary data.

## Stage 4 — Networking

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

Acceptance criteria:

- [ ] GenOS obtains an address through DHCP.
- [ ] ICMP echo works against the QEMU host network.
- [ ] A userspace program resolves DNS and completes an HTTP request.
- [ ] Malformed-packet tests do not panic or corrupt memory.
- [ ] Packet loss and connection timeout behavior are defined.

## Stage 5 — Security and identity

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

## Stage 6 — Application and graphics platform

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

## Stage 7 — Hardware and daily-use expansion

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
