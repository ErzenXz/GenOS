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

**Status: next**

Goal: move from kernel-owned demo tasks to isolated executable processes.

Planned work:

- privilege transition to ring 3;
- per-process address spaces;
- virtual-memory mappings and page-fault handling;
- syscall entry/exit path;
- preemptive scheduler with sleep and wake primitives;
- userspace ELF loader;
- process lifecycle and exit status;
- pipes or bounded message channels;
- initial userspace runtime crate;
- move the shell into userspace.

Acceptance criteria:

- [ ] Two independent userspace programs run with separate address spaces.
- [ ] A userspace crash terminates only the failing process.
- [ ] The scheduler demonstrates preemption rather than cooperative polling.
- [ ] Syscall arguments are validated before kernel access.
- [ ] The shell can launch, inspect, and terminate a userspace process.
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
