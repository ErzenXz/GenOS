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
- ABI 15 userspace coordination, capability-based VFS I/O, namespace mutation, messaging, console access, process lifecycle control, and bounded UDP/TCP exchanges;
- application output copied from validated user mappings into the serial shell;
- a DPL3 `int 0x80` syscall gate with scalar and user-buffer validation before copy-in;
- COM1 input/output with userspace-owned terminal editing;
- host-driven serial command testing through the real Ring 3 input path;
- bounded kernel-worker lifecycle with PIDs, protected system tasks, and slot reuse;
- round-robin scheduling with measured CPU slices, sleep/wake deadlines, and context-switch accounting;
- writable RAM-backed virtual filesystem;
- PCI-discovered ATA storage, MBR partitions, write-back caching, dual-generation durable snapshots, repair tooling, and read-only recovery;
- NE2000, Ethernet, ARP, IPv4/ICMP, UDP/DHCP/DNS, TCP, and ABI 15 Ring 3 network exchanges;
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

GenOS 0.42 boots `SHELL.ELF` as a long-lived Ring 3 process with its own CR3, guarded stack, runtime identity, and opaque console, VFS, lifecycle, and network access. COM1 bytes wake that process through the normal input path. The shell owns its eight-command history and executes `help`, `echo`, `uname`, `net`, `clear`, `ls`, `cat`, `stat`, `touch`, `write`, `append`, `mkdir`, `rm`, `run init [hold]`, `ps`, `kill JOB`, and `wait JOB`. ABI 15 preserves the capability-based filesystem and process contracts while adding validated, bounded UDP and TCP exchanges.

Normal filesystem and process-control commands now stay out of Ring 0. If the Ring 3 shell is not live, a separate emergency parser accepts only `help`, `status`, `mem`, `reboot`, and `shutdown`; every normal shell command is rejected at the kernel boundary.

The Stage 3 runtime coordinator now owns task scheduling, process polling, lifecycle launches, VFS completions, and their pending queues independently of the display. `ProcessManager` is the sole userspace lifecycle authority; Task Manager renders immutable snapshots composed from process state and kernel-worker accounting. Every handle and deferred request is bound to its exact caller and operation. Stale, canceled, and replayed work is rejected before external mutation. The shell is the session supervisor: its exit, fault, or kill terminates and reaps every owned child, cancels pending service work, revokes all authority, and removes child task rows before the shell becomes terminal. Boot proofs exercise these boundaries before the framebuffer exists. See [the runtime ownership notes](docs/RUNTIME.md) for the exact policy and remaining cleanup.

Stage 3 closes with transactional failure cleanup, 257 sequential real process launches across PID reuse, and a fresh-QEMU transcript through the actual Ring 3 console path. Stage 4 is complete in GenOS 0.41: PCI discovers the IDE controller, a bounds-checked MBR partition holds alternating 20 KiB `GFS2` snapshots, and an eight-sector write-back cache flushes successful `/USER/` mutations before returning success. The host can inspect or conservatively repair an image, torn generations fall back safely, fully corrupt media is surfaced to Ring 3, explicit read-only recovery preserves files while denying mutation, and `/TMP/SESSION.TXT` remains session-only. See [the storage notes](docs/STORAGE.md) for the exact format and recovery policy.

Stage 5 is complete in GenOS 0.42. The NE2000 path obtains DHCP configuration, resolves next hops with ARP, echoes ICMP, resolves DNS over UDP, and completes a TCP/HTTP exchange from Ring 3. Parsers reject malformed and checksum-invalid packets, and all DHCP, ARP, UDP, and TCP waits have a three-attempt bounded failure policy. See [the networking notes](docs/NETWORKING.md) for the exact protocol and ABI scope.

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
    |-- input                    PS/2 queue / filters / desktop-or-Ring-3 routing
    |-- storage                  MBR + ATA cache + durable /USER snapshots + RAM /TMP
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
- [x] **Networking:** NE2000, packet ownership, Ethernet, ARP, IPv4/ICMP, UDP/DHCP/DNS, TCP, Ring 3 exchanges, and diagnostics
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
