# GenOS

**A small, understandable operating system built to become fast, focused, and genuinely pleasant to use.**

[![CI](https://github.com/ErzenXz/GenOS/actions/workflows/ci.yml/badge.svg)](https://github.com/ErzenXz/GenOS/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-c9a252.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-x86__64-565854.svg)](#build-and-run)
[![Stage](https://img.shields.io/badge/stage-experimental-d6a752.svg)](#project-status)

GenOS is a from-scratch `x86_64` operating system written in Rust. It boots through its own UEFI loader, enters a `no_std` kernel, initializes memory and interrupts, mounts durable storage, configures an IPv4 network, and opens a serial terminal backed by an isolated Ring 3 shell. The current build is intentionally server-first and runs without a framebuffer UI.

The long-term goal is ambitious: build an operating system that feels lighter than Windows, more coherent than a typical desktop Linux installation, and simple enough that a curious developer can understand the path from power-on to pixel.

GenOS is **not a Linux distribution** and does not use the Linux kernel. It is also not ready to replace a daily-driver operating system today. The current release is an experimental foundation designed to grow in public.

## Why GenOS?

Modern operating systems are extraordinarily capable, but decades of compatibility requirements, layered abstractions, duplicated services, and product compromises have made them difficult to understand and expensive to change.

GenOS starts from a smaller set of principles:

- **Small before clever.** Every subsystem should justify its complexity.
- **One coherent product.** Kernel, desktop, applications, and developer tooling should evolve together.
- **Fast by construction.** Avoid unnecessary background work, copies, allocations, and abstraction layers.
- **Understandable internals.** A contributor should be able to trace an input event or rendered frame without crossing dozens of repositories.
- **Safe foundations.** Rust removes broad classes of memory errors while still allowing precise low-level control.
- **Useful progress.** Each milestone should produce something observable, testable, and worth demonstrating.
- **Honest scope.** GenOS will earn capability through measured milestones instead of pretending an experiment is already production-ready.

## What are we trying to improve?

GenOS is inspired by the strengths of Windows, Linux, macOS, BSD, and research operating systems. It is not built around insulting those projects; it is built around learning from the trade-offs they expose.

### Where Windows can feel heavy

Windows carries an enormous compatibility contract across hardware generations, enterprise environments, application models, and decades of software. That strength also creates visible costs:

- large background-service and update footprints;
- inconsistent interfaces produced by multiple generations of system UI;
- opaque system activity that can be difficult to explain or control;
- a growing baseline for memory, storage, and hardware requirements;
- product decisions that do not always align with a user's desire for a quiet, local-first computer.

GenOS explores the opposite starting point: a small baseline, explicit system activity, and a desktop built alongside the kernel rather than layered over decades of compatibility.

### Where desktop Linux can feel fragmented

Linux offers extraordinary freedom, performance, hardware reach, and engineering quality. Its modular ecosystem is also its defining desktop challenge:

- distributions, package formats, desktop environments, display stacks, and configuration models can diverge;
- polished behavior may depend on integration work spread across many independent projects;
- common desktop tasks sometimes require knowledge of the underlying system;
- application distribution and hardware support can vary significantly between installations;
- there is no single product team responsible for the entire user experience.

GenOS keeps the openness and inspectability, but aims for a single reference system with one documented application model, one visual language, and one release path.

### The GenOS bet

The bet is that a focused system can eventually deliver:

1. a very small, measurable resource baseline;
2. predictable behavior with fewer invisible services;
3. a coherent desktop and application platform;
4. strong isolation without making the system impossible to understand;
5. an approachable codebase that helps new systems programmers learn and contribute.

That outcome will take years of careful work. This repository is the starting point.

## What works today

The current GenOS build already contains real operating-system infrastructure:

- repo-owned `x86_64` UEFI bootloader;
- ELF kernel loading and versioned boot information;
- Rust `no_std` kernel with GDT, TSS, IDT, and interrupt setup;
- gap-safe physical frame allocation that keeps reserved firmware ranges out of circulation;
- cloned supervisor-only kernel page tables with explicitly exposed user code and stack pages;
- three ring-3 process instances with separate CR3 roots and private code, data, guard, and stack mappings;
- timer-driven CPU preemption with saved contexts, address-space switching, and resume;
- process-local user page-fault termination that leaves healthy processes and the kernel running;
- a bounded ELF64 parser and W^X userspace segment loader;
- a separately built `no_std` userspace runtime plus packaged `INIT.ELF` and `SHELL.ELF` applications;
- boot-time and shell-triggered ELF launches, each receiving a fresh address space;
- asynchronous userspace scheduling with observable ready, sleeping, waiting, exited, faulted, and killed states;
- `wait`/`kill` lifecycle controls and deterministic address-space frame reclamation;
- ABI 17 userspace coordination, capability-based VFS I/O, namespace mutation, messaging, console access, process lifecycle control, bounded UDP/TCP exchanges, non-blocking socket capabilities, and TCP listener authority;
- application output copied from validated user mappings into the serial shell;
- a DPL3 `int 0x80` syscall gate with scalar and user-buffer validation before copy-in;
- COM1 input/output with userspace-owned terminal editing;
- host-driven serial command testing through the real Ring 3 input path;
- bounded kernel-worker lifecycle with PIDs, protected system tasks, and slot reuse;
- round-robin scheduling with measured CPU slices, sleep/wake deadlines, and context-switch accounting;
- writable RAM-backed virtual filesystem;
- PCI-discovered ATA storage, MBR partitions, write-back caching, dual-generation durable snapshots, repair tooling, and read-only recovery;
- modern-only VirtIO 1.x PCI networking with split RX/TX virtqueues, bounded DMA buffers, a legacy recovery fallback, Ethernet, ARP, IPv4/ICMP, UDP/DHCP/DNS, TCP, ABI 15 exchange compatibility, asynchronous ABI 16 UDP/TCP clients, and ABI 17 listener capabilities with one bounded passive handshake;
- serial boot diagnostics and long-running headless QEMU smoke tests.

The build has no host operating-system runtime underneath it. QEMU provides virtual hardware, but the bootloader, kernel, input path, filesystem, task model, drawing, and desktop behavior are GenOS code.

## Serial terminal

| Action | Control |
| --- | --- |
| Start GenOS | Run `cargo xtask run` |
| Enter a command | Type over the QEMU serial console and press Enter |
| Edit input | Use printable characters and Backspace |
| List shell commands | Run `help` |
| Show network status | Run `net` |

GenOS 0.49 boots `SHELL.ELF` as a long-lived Ring 3 process with its own CR3, guarded stack, runtime identity, and opaque console, VFS, lifecycle, and network access. COM1 bytes wake that process through the normal input path. The shell owns its eight-command history and executes `help`, `echo`, `uname`, `net`, `clear`, `ls`, `cat`, `stat`, `touch`, `write`, `append`, `mkdir`, `rm`, `run init [hold]`, `ps`, `kill JOB`, and `wait JOB`. ABI 17 preserves generation-safe UDP/TCP client objects, exclusive TCP port binding, bounded listener backlogs, non-blocking accept, and accepted-child capabilities. One exact listener can now complete a bounded passive handshake, receive one bounded request through the accepted Ring 3 handle, return one bounded response, and complete an orderly half-close.

Normal filesystem and process-control commands now stay out of Ring 0. If the Ring 3 shell is not live, a separate emergency parser accepts only `help`, `status`, `mem`, `reboot`, and `shutdown`; every normal shell command is rejected at the kernel boundary.

The Stage 3 runtime coordinator now owns task scheduling, process polling, lifecycle launches, VFS completions, and their pending queues independently of the display. `ProcessManager` is the sole userspace lifecycle authority; Task Manager renders immutable snapshots composed from process state and kernel-worker accounting. Every handle and deferred request is bound to its exact caller and operation. Stale, canceled, and replayed work is rejected before external mutation. The shell is the session supervisor: its exit, fault, or kill terminates and reaps every owned child, cancels pending service work, revokes all authority, and removes child task rows before the shell becomes terminal. Boot proofs exercise these boundaries before the framebuffer exists. See [the runtime ownership notes](docs/RUNTIME.md) for the exact policy and remaining cleanup.

Stage 3 closes with transactional failure cleanup, 257 sequential real process launches across PID reuse, and a fresh-QEMU transcript through the actual Ring 3 console path. Stage 4 is complete in GenOS 0.41: PCI discovers the IDE controller, a bounds-checked MBR partition holds alternating 20 KiB `GFS2` snapshots, and an eight-sector write-back cache flushes successful `/USER/` mutations before returning success. The host can inspect or conservatively repair an image, torn generations fall back safely, fully corrupt media is surfaced to Ring 3, explicit read-only recovery preserves files while denying mutation, and `/TMP/SESSION.TXT` remains session-only. See [the storage notes](docs/STORAGE.md) for the exact format and recovery policy.

Stage 5.1 is complete in GenOS 0.43. QEMU now exposes only the modern VirtIO 1.x PCI interface by default; GenOS negotiates `VIRTIO_F_VERSION_1`, configures independent split RX/TX virtqueues, and transfers frames through bounded DMA-visible buffers. The protocol stack sits behind a frame-device boundary, with NE2000 retained only as an explicitly labelled legacy recovery fallback. DHCP, ARP, ICMP, DNS, TCP, and the Ring 3 HTTP/1.1 proof all run on VirtIO, and the smoke test requires the exact modern-driver marker so fallback cannot produce a false pass. See [the networking notes](docs/NETWORKING.md) for the exact contract and remaining socket, IPv6, TCP, and TLS milestones.

Stage 5.2 is complete in GenOS 0.44. ABI 16 socket handles are typed, process-local, generation-safe objects with fixed queue budgets, observable readiness, `WOULD_BLOCK` backpressure, partial receive preservation, half-close state, close revocation, and process-exit cleanup. Ring 3 proves the lifecycle and authority checks during every modern-network smoke boot.

Stage 5.3 is complete in GenOS 0.45. The runtime moves one UDP datagram at a time from a process queue into an exact request identity, then performs bounded ARP resolution, transmission, response demultiplexing, retry, timeout, and cancellation outside the syscall. Ring 3 resolves DNS through the ABI 16 socket itself, and both network boots require transport-start, completion, timeout, and stale-request cancellation markers. At that milestone boundary TCP queue progress, listeners, multi-socket fairness, interrupt-driven VirtIO completion, production loss recovery, IPv6, and TLS were deliberately left open.

Stage 5.4A is complete in GenOS 0.46. ABI 16 TCP client requests now progress through ARP, SYN/SYN-ACK, request data, bounded ordered response, ACK, FIN, reset, retry, timeout, and cancellation states outside the syscall. The deterministic QEMU server accepts both the new socket transaction and the retained ABI 15 compatibility exchange; the modern-network gate requires the TCP socket response to reach Ring 3, while the no-server gate requires clean RST failure. At that milestone boundary listener authority and all server-side wire behavior remained open.

Stage 5.4B is complete in GenOS 0.47. ABI 17 adds capability-scoped `socket_bind`, `socket_listen`, and non-blocking `socket_accept`, exclusive local-port ownership across live processes, a fixed two-connection backlog budget, generation-safe accepted-child objects, typed-handle rollback, and close/exit port release. Ring 3 proves low-port denial, forged-handle denial, empty accept, duplicate-bind refusal, and close/rebind cleanup on every boot. At that milestone boundary the listener was an authority foundation only; Stage 5.4C adds the first wire-backed backlog admission while concurrent host clients and long-lived accepted streams remain open work.

Stage 5.4C is complete in GenOS 0.48. The scheduler-driven receive path validates an exact checksum-protected IPv4/TCP SYN for a live bound listener, sends a bounded SYN-ACK with retry and timeout, validates the final ACK tuple and sequence numbers, and admits the peer through that listener's fixed backlog. The deterministic QEMU gate connects through host forwarding and requires the passive SYN, completed handshake, and Ring 3 accepted-capability markers; the ordinary no-host boot must still complete. At that milestone boundary accepted children were deliberately non-writable; Stage 5.4D adds the first bounded stream transaction.

Stage 5.4D is complete in GenOS 0.49. The established peer's MAC, address, ports, and initial sequence state now travel unchanged through the listener backlog into the accepted capability. The runtime admits an exact ACK-protected 10-byte request into the socket receive queue, binds the 10-byte response to a nonzero request identity, retries until its exact ACK, handles the peer half-close, sends FIN, and waits for wire acknowledgment before exposing `Closed`. QEMU verifies `GENOS_PING` → `GENOS_PONG` plus EOF through the real Ring 3 handle. This remains one statically bounded transaction and one passive stream; simultaneous clients, arbitrary long-lived byte streams, production recovery, and fair service are still open.

GenOS 0.16 makes messaging a capability instead of a PID. A process publishes exactly one receive endpoint with `create_endpoint`; every other process must obtain its own send handle with `connect_endpoint` before it can send anything, and holding a raw PID grants nothing. Handles are opaque values that encode a dedicated endpoint tag, the owner PID, a per-process generation, and a slot in a four-entry table, so a guessed, foreign, or stale handle is rejected rather than resolved. The kernel fills in the sender PID of every delivered message, so Ring 3 cannot forge an identity. Each endpoint queue holds four messages and admits at most one per producer, which is what keeps a single noisy producer from starving its peers. A receive on an empty queue leaves the runnable set entirely and is woken by a later send that copies straight into its already-validated buffer. Closing the receive handle unpublishes the endpoint and revokes every remote send handle naming that generation; normal exit, a Ring 3 fault, `kill`, and reap run the same release path.

`run fanin` is the real three-process proof. A receiver publishes an endpoint and sleeps while two independent producer children connect to it. Producer A sends `A1` and its immediate second send is refused with `USER_ERROR_UNAVAILABLE` because `A1` is still queued; producer B then sends `B1`. The receiver drains `A1` and `B1` in arrival order, parks on an empty queue for its third receive, and is woken directly by producer A's retried `A2` — output `INIT.ELF fan-in A1 B1 A2`. The boot proof requires exactly three delivered messages, exactly one fairness denial, exactly one direct wake, and reclamation of all three address spaces. See [the userspace boundary notes](docs/USERSPACE.md) for the exact syscall contract, guarantees, and limitations.

## Architecture

```text
UEFI firmware
    |
    v
GenOS bootloader
    |  loads kernel ELF + initrd
    v
GenOS kernel
    |-- architecture setup       GDT / TSS / IDT / IRQ
    |-- memory                   gap-safe + recycled frames / protected page tables
    |-- ELF loader              bounded parser / W^X segment mapping
    |-- userspace               async lifecycle / file I/O / endpoints / Ring 3 shell
    |-- runtime                 scheduler / process table / VFS service queues
    |-- input                    bounded event queues / current PS/2 fallback transport
    |-- storage                  durable /USER snapshots + RAM /TMP / current ATA fallback
    |-- networking               VirtIO 1.x PCI queues / bounded frame-device boundary
    |-- tasks                    registry / state / accounting
    |-- display                  backbuffer / dirty regions / text
    `-- desktop                  windows / apps / taskbar / runtime-update renderer
```

The system remains intentionally monolithic while its contracts are established, but the hardware-enforced user/kernel boundary now supports a small interactive application lifecycle. It is still an experimental runtime, not a compatibility layer for existing Linux or Windows applications.

## Project status

GenOS is an **experimental developer preview**.

It is suitable for:

- learning about UEFI and kernel development;
- experimenting with low-level Rust;
- contributing to an early operating-system architecture;
- testing desktop and graphics ideas in a controlled environment.

It is not yet suitable for:

- storing important or persistent data;
- running existing Windows or Linux applications;
- daily-driver desktop use;
- deployment on untested physical hardware;
- environments requiring a mature security model.

See [ROADMAP.md](ROADMAP.md) for milestone definitions, acceptance criteria, and the path toward userspace, persistent storage, networking, security, and an application ecosystem.

## Build and run

### Requirements

- Rust 1.93 or newer
- `x86_64-unknown-uefi` and `x86_64-unknown-none` Rust targets
- QEMU with EDK2/OVMF firmware
- `mtools`

On macOS with Homebrew:

```sh
brew install qemu mtools
rustup target add x86_64-unknown-uefi x86_64-unknown-none
```

On Ubuntu or Debian:

```sh
sudo apt-get install qemu-system-x86 ovmf mtools
rustup target add x86_64-unknown-uefi x86_64-unknown-none
```

### Commands

```sh
# Build the bootable disk image at build/genos.img
make build

# Boot GenOS in QEMU
make run

# Run unit tests, rebuild, and execute the long-lived QEMU smoke test
make test

# Remove generated build artifacts
make clean
```

A successful boot writes `SERVER_TERMINAL_READY`, `SERIAL_TERMINAL_READY`, and `GENOS_READY` to the serial console, then presents the `genos>` prompt without opening a graphical display.

## Repository layout

```text
bootloader/       UEFI loader and ELF loading
crates/abi/       Versioned bootloader-kernel contract
kernel/           no_std kernel, hardware, filesystem, and desktop
userspace/         no_std runtime and independently linked ELF application
tools/xtask/      Build image, initrd, QEMU, and smoke-test automation
.github/          CI and contribution workflows
```

## Roadmap at a glance

- [x] **Foundation:** UEFI boot, kernel entry, framebuffer, serial diagnostics
- [x] **Interactive desktop:** input, windows, shell, RAM filesystem, live task UI
- [x] **Processes and userspace:** private address spaces, Ring 3, ELF loading, preemption, capability-based file, directory, endpoint, console, and lifecycle I/O, plus a persistent Ring 3 shell with safe `/USER/` file mutation and job control
- [x] **Runtime cleanup and console-first system:** desktop-independent services, one lifecycle authority, unified handles, and a composable command-line userland
- [x] **Persistent storage:** PCI-discovered ATA block I/O, MBR partitioning, write-back caching, durable `/USER/` snapshots, conservative repair, and read-only recovery
- [x] **Modern network transport:** VirtIO 1.x PCI, split queues, bounded DMA ownership, Ethernet, ARP, IPv4/ICMP, UDP/DHCP/DNS, TCP, Ring 3 exchanges, and diagnostics
- [x] **Socket capability foundation:** ABI 17 generation-safe handles, bounded queues, readiness, backpressure, shutdown, cancellation, listener authority, and Ring 3 proof
- [x] **Asynchronous UDP sockets:** exact request identity, scheduler-driven ARP/UDP, Ring 3 DNS, bounded timeout, and cancellation
- [x] **Asynchronous TCP client sockets:** scheduler-driven handshake/request/response, exact completion, RST handling, retry, timeout, and cancellation
- [x] **Passive TCP admission:** exact listener lookup, bounded SYN/SYN-ACK/ACK, backlog admission, and real Ring 3 accept
- [x] **Bounded accepted stream:** exact request receive, response ACK, peer half-close, guest FIN, and Ring 3 close proof
- [ ] **Mature networking:** long-lived streams, fair concurrent service, interrupt-driven completion, production congestion/loss behavior, IPv6 dual stack, TLS 1.3, and HTTPS
- [ ] **Security model:** identities, capabilities, isolation, secure update design
- [ ] **Application platform:** stable SDK, packages, compositor, richer graphics
- [ ] **Hardware expansion:** ACPI, SMP, USB, NVMe, audio, broader GPU support

Progress is accepted through working code and measurable criteria, not roadmap labels alone. The detailed plan lives in [ROADMAP.md](ROADMAP.md).

## Performance philosophy

“Lightweight” needs numbers. As GenOS grows, the project will publish and track:

- boot time to a usable serial terminal;
- idle memory footprint;
- idle wakeups and CPU time;
- input-to-frame latency;
- binary and disk-image size;
- filesystem and network throughput;
- regression budgets for every release.

The project will prefer evidence over claims. GenOS should only call itself faster or lighter when repeatable benchmarks demonstrate it.

The boot smoke test now records scheduler evidence directly. `SCHED_DISPATCH_BENCH` reports ready-to-dispatch latency for the round-robin worker policy in ticks, while `SCHED_CONTEXT_BENCH` reports 32 measured kernel-to-process-to-kernel CR3 switch pairs in CPU cycles. These numbers cover scheduler policy and address-space switching only; they are not end-to-end application latency.

## Contributing

GenOS is early enough that thoughtful contributors can still shape its architecture.

Good first areas include tests, documentation, shell ergonomics, framebuffer primitives, filesystem correctness, build portability, and hardware emulation coverage. Larger changes to scheduling, userspace, storage, networking, or the application ABI should begin with an issue describing the contract and migration path.

Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request. Please follow the [Code of Conduct](CODE_OF_CONDUCT.md) and report security concerns through the process in [SECURITY.md](SECURITY.md).

## Principles for pull requests

- Keep the system bootable after every change.
- Prefer a small complete subsystem over a broad placeholder.
- Add a serial marker or test when introducing a boot-critical path.
- Explain unsafe code and keep its scope narrow.
- Do not hide major architectural decisions inside unrelated patches.
- Preserve the `no_std` kernel boundary.
- Measure performance-sensitive changes.

## License

GenOS is released under the [MIT License](LICENSE).

## A note on the ambition

Could GenOS become lighter and more coherent than today's mainstream systems? Yes—that is the reason to build it.

Is it already “better than Linux” or ready to replace Windows? No. Linux and Windows represent decades of engineering across thousands of devices and workloads. GenOS will respect that reality, learn in public, and compete one proven milestone at a time.

If that mission sounds worthwhile, build it, test it, challenge the design, and help move the roadmap forward.
