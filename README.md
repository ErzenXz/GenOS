# GenOS

**A small, understandable operating system built to become fast, focused, and pleasant to use.**

[![CI](https://github.com/ErzenXz/GenOS/actions/workflows/ci.yml/badge.svg)](https://github.com/ErzenXz/GenOS/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-c9a252.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-x86__64-565854.svg)](#build-and-run)
[![Stage](https://img.shields.io/badge/stage-experimental-d6a752.svg)](#project-status)

GenOS is a from-scratch `x86_64` operating system written in Rust. It boots through its own UEFI loader, enters a `no_std` kernel, creates isolated Ring 3 processes, mounts bounded persistent storage, configures a VirtIO IPv4 network, and opens a serial terminal backed by a userspace shell.

GenOS is **not a Linux distribution** and does not use the Linux kernel. QEMU supplies virtual hardware, not an operating-system runtime. The bootloader, kernel, process model, syscalls, shell, storage, and network stack in the image are GenOS code.

## Project status

GenOS is an **experimental developer preview**. The active product path is serial-first and intentionally runs without a framebuffer UI.

The current system is useful for:

- learning and experimenting with UEFI, x86-64, Rust, paging, interrupts, processes, capabilities, storage, and networking;
- testing early operating-system contracts in a controlled virtual machine;
- contributing to an architecture while its compatibility commitments remain small.

It is not suitable for:

- important or irreplaceable data;
- secrets or hostile workloads;
- production services;
- daily-driver use;
- untested physical hardware;
- existing Linux or Windows applications;
- environments that require a maintained security or compatibility contract.

Read the [known limitations](docs/KNOWN_LIMITATIONS.md) before treating a demonstrated feature as a supported one.

## Why build another operating system?

Mainstream operating systems solve enormous compatibility and hardware problems. GenOS explores a different starting point:

- **Small before clever.** Every subsystem must justify its complexity.
- **One coherent product.** Kernel, userspace, applications, and tools evolve together.
- **Explicit authority.** A PID, path, port, or internal slot does not grant access by itself.
- **Transactional failure.** Partial construction and mutation must roll back cleanly.
- **Measured performance.** “Fast” and “lightweight” require reproducible numbers.
- **Understandable internals.** A contributor should be able to trace ownership from entry to cleanup.
- **Honest scope.** A narrow experiment is described as a narrow experiment.

The long-term goal is to build a system that can beat larger platforms on defined workloads while remaining inspectable and coherent. It is not credible to claim universal superiority over Linux, Windows, macOS, or BSD. GenOS will compete one published metric and one proven contract at a time.

## What works today

The GenOS 0.49 experimental baseline includes:

### Boot and architecture

- repo-owned `x86_64` UEFI bootloader;
- ELF kernel loading and a versioned boot-information contract;
- Rust `no_std` monolithic kernel;
- GDT, TSS, IDT, PIT/PIC interrupt setup, and serial diagnostics;
- gap-safe physical frame discovery across firmware memory-map holes;
- supervisor page-table cloning and isolated user address-space roots.

### Processes and authority

- separately linked `INIT.ELF` and `SHELL.ELF` Ring 3 applications;
- private code, data, guarded stack, saved context, and CR3 per process;
- timer-driven preemption and bounded round-robin scheduling;
- process-local page-fault containment for the currently handled user faults;
- ABI 17 through a DPL3 `int 0x80` gate with scalar and user-buffer validation;
- one authoritative typed handle table per process;
- generation-safe file, directory, endpoint, console, lifecycle, process, and socket capabilities;
- exact asynchronous request identity and stale-completion rejection;
- exit, fault, kill, cancellation, and reap cleanup paths;
- an isolated Ring 3 serial shell with file and job-control commands;
- a fail-closed emergency Ring 0 console limited to diagnostics and power control.

### Storage

- writable RAM-backed temporary filesystem;
- PCI-discovered ATA reference path;
- bounds-checked MBR partition access;
- fixed write-back cache;
- alternating bounded persistent filesystem generations for `/USER/`;
- image inspection, conservative repair, torn-generation fallback, corruption reporting, and read-only recovery.

### Networking

- modern VirtIO 1.x PCI negotiation and split RX/TX virtqueues;
- bounded DMA-visible buffers behind a frame-device boundary;
- Ethernet, ARP, IPv4, ICMP, UDP, DHCP, DNS, and TCP parsing and state;
- generation-safe UDP and TCP socket capabilities;
- scheduler-driven bounded UDP and TCP client transactions;
- exclusive TCP listener authority and bounded backlog;
- one passive handshake and one accepted request/response/close transaction;
- deterministic host-side and QEMU network proofs.

### Verification

- host unit tests for core bounded state and parser behavior;
- serial boot diagnostics;
- QEMU smoke tests through real Ring 3 and device paths;
- scheduler and CR3-switch measurements for their narrowly defined paths.

### Important qualification

The ELF loader rejects writable-and-executable load segments. That is not yet the same as proven system-wide W^X. The audited baseline adds the page-table NX bit only when firmware has already enabled `EFER.NXE`; the foundation roadmap requires GenOS to enable and test NX, supervisor write protection, SMEP, and SMAP itself.

The current system also remains single-core, uses a fixed recycled-frame pool, has incomplete exception coverage, runs development stress probes during normal boot, and contains concentrated userspace kernel code. These are release blockers, not cosmetic cleanup.

## Immediate engineering priority

Before broad product expansion, GenOS is closing a foundation correctness gate:

1. complete and normalize every architectural exception entry;
2. enable and prove CPU page-protection features;
3. make physical and virtual memory construction transactional and scalable;
4. split runtime responsibilities into reviewable modules;
5. formalize the single-core concurrency model and per-CPU path;
6. separate validation boot from release boot;
7. add fuzzing, fault injection, long-run tests, and reviewable delivery rules;
8. keep every required CI stage green.

The full acceptance criteria live in [ROADMAP.md](ROADMAP.md). The testing and evidence model lives in [docs/ENGINEERING_QUALITY.md](docs/ENGINEERING_QUALITY.md).

## Architecture

```text
UEFI firmware
    |
    v
GenOS bootloader
    |  loads kernel ELF + initrd + versioned boot data
    v
GenOS kernel
    |-- architecture      GDT / TSS / IDT / interrupts / privilege entry
    |-- memory            physical frames / page tables / user mappings
    |-- process           Ring 3 contexts / lifecycle / capabilities
    |-- runtime           scheduling / requests / VFS and network completion
    |-- storage           RAM VFS / persistent snapshots / block transport
    |-- networking        VirtIO queues / Ethernet / IPv4 / UDP / TCP
    |-- input             serial and current PS/2 fallback transport
    `-- diagnostics       serial markers / recovery console / measurements
          |
          v
    Ring 3 userspace
    |-- runtime library
    |-- INIT.ELF
    `-- SHELL.ELF
```

The kernel remains intentionally monolithic while contracts are established. “Monolithic” does not mean one source file or unrestricted mutation. The roadmap requires explicit subsystem ownership and narrow interfaces before SMP and broader feature work.

## Serial terminal

| Action | Control |
| --- | --- |
| Start GenOS | `make run` or `cargo xtask run` |
| Enter a command | Type over the QEMU serial console and press Enter |
| Edit input | Printable characters and Backspace |
| List commands | `help` |
| Show network state | `net` |

The Ring 3 shell supports commands including `help`, `echo`, `uname`, `net`, `clear`, `ls`, `cat`, `stat`, `touch`, `write`, `append`, `mkdir`, `rm`, `run init [hold]`, `ps`, `kill JOB`, and `wait JOB`.

A successful reference boot writes `SERVER_TERMINAL_READY`, `SERIAL_TERMINAL_READY`, and `GENOS_READY`, then presents the `genos>` prompt.

## Build and run

### Requirements

- Rust 1.93 or newer;
- `x86_64-unknown-uefi` and `x86_64-unknown-none` Rust targets;
- QEMU with EDK2/OVMF firmware;
- `mtools`.

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
# Build build/genos.img
make build

# Boot the image in QEMU
make run

# Run workspace tests, rebuild, and execute the QEMU smoke suite
make test

# Run focused static checks
cargo fmt --all -- --check
cargo clippy -p kernel --lib -- -D warnings
cargo test --workspace

# Remove generated artifacts
make clean
```

## Repository layout

```text
bootloader/       UEFI loader and kernel ELF loading
crates/abi/       Versioned bootloader-kernel and userspace contracts
kernel/           no_std kernel, architecture, runtime, storage, and network
userspace/        no_std runtime and independently linked applications
tools/xtask/      Image, initrd, QEMU, and smoke-test automation
docs/             Subsystem contracts, limitations, quality plan, and ADRs
.github/          CI, issue forms, and contribution workflow
```

## Documentation

- [Roadmap and acceptance gates](ROADMAP.md)
- [Engineering quality plan](docs/ENGINEERING_QUALITY.md)
- [Known limitations](docs/KNOWN_LIMITATIONS.md)
- [Userspace boundary and ABI](docs/USERSPACE.md)
- [Runtime ownership](docs/RUNTIME.md)
- [Storage format and recovery](docs/STORAGE.md)
- [Networking contracts](docs/NETWORKING.md)
- [Architecture decision records](docs/adr/README.md)
- [Contribution guide](CONTRIBUTING.md)
- [Security policy](SECURITY.md)

## Roadmap at a glance

- [x] Experimental UEFI, kernel, serial, desktop, userspace, runtime, storage, and bounded IPv4/TCP vertical slices
- [ ] Foundation correctness gate: traps, protection, memory rollback, ownership, concurrency, test modes, and green verification
- [ ] Concurrent production-oriented TCP behavior and interrupt-driven VirtIO
- [ ] IPv6 dual stack
- [ ] Security, identity, cryptographic trust, signed packages, and updates
- [ ] Stable application and service platform
- [ ] APIC/MSI-X, SMP, NVMe, xHCI, IOMMU, power management, and reference hardware
- [ ] Userspace compositor and coherent desktop product
- [ ] Hardened preview and daily-use qualification

Delivered checkboxes describe experimental slices. They do not supersede the open foundation gate.

## Performance and comparisons

GenOS tracks performance only where the experiment is reproducible. Initial metrics include:

- firmware entry to usable serial prompt;
- idle allocated frames, memory, wakeups, and CPU time;
- kernel, application, and disk-image size;
- syscall, process lifecycle, scheduler, and address-space-switch latency;
- storage commit, restore, and recovery;
- UDP and TCP latency, throughput, recovery, CPU cost, and queue occupancy;
- later, input-to-frame and compositor latency.

A comparison with Linux or another system must pin both configurations, use the same hardware or VM definition, publish the workload and raw samples, report variance and failures, and state missing features that affect the result. One benchmark result applies only to that experiment.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md), the [roadmap](ROADMAP.md), the [quality plan](docs/ENGINEERING_QUALITY.md), and the [known limitations](docs/KNOWN_LIMITATIONS.md) before changing a kernel contract.

Good changes are focused, bootable, measurable, and explicit about authority, unsafe assumptions, failure, cleanup, compatibility, and rollback. Major scheduling, memory, interrupt, ABI, storage, networking, security, driver, or package decisions should begin with an architecture proposal and end with an architecture decision record.

Report security concerns through the private process in [SECURITY.md](SECURITY.md).

## License

GenOS is released under the [MIT License](LICENSE).

## The ambition

GenOS can become lighter, easier to inspect, and more coherent than larger systems for selected workloads. Reaching that point requires stronger fundamentals, not louder claims. The project will publish what it proves, record what it does not, and improve one reviewable change at a time.