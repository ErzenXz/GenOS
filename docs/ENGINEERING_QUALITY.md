# GenOS engineering quality plan

This document defines how GenOS turns ambitious goals into verifiable engineering claims. It complements the [roadmap](../ROADMAP.md) and the [known limitations register](KNOWN_LIMITATIONS.md).

GenOS is currently an experimental operating system. The project should move quickly, but it must not make progress by hiding uncertainty, skipping failure paths, or calling a narrow demonstration production-ready.

## Quality model

GenOS evaluates quality across seven independent dimensions:

1. **Correctness:** the implementation obeys its documented contract in success and failure paths.
2. **Security:** authority is explicit, isolation is enforced by hardware and software, and malformed input fails closed.
3. **Reliability:** recovery is deterministic, resources are reclaimed, and long-running operation does not accumulate corruption.
4. **Performance:** resource and latency claims come from reproducible measurements.
5. **Maintainability:** ownership, unsafe invariants, compatibility, and review scope remain understandable.
6. **Hardware behavior:** drivers validate device state, DMA ownership, timeout, reset, and removal.
7. **Product behavior:** users receive predictable interfaces, recovery paths, accessibility, and upgrade policy.

A release cannot average these dimensions together. Excellent performance does not cancel a memory-safety failure. A polished interface does not cancel an isolation failure.

## Release levels

| Level | Meaning | Minimum gate |
| --- | --- | --- |
| Experimental | A development image that demonstrates selected mechanisms | Builds and boots on the documented development configuration |
| Verified reference build | The current reference VM passes the complete foundation correctness gate | Roadmap F0-F7 complete |
| Hardened preview | Security boundaries, fuzzing, fault injection, long-run tests, signed distribution, and supported configurations are published | Foundation gate plus Stage 6 criteria |
| Production candidate | Upgrade, rollback, compatibility, hardware, support, and independent reproduction requirements pass | Stages 7-10 criteria |

The current project level is **Experimental**.

## Non-negotiable invariants

These invariants apply before feature-specific acceptance criteria.

### Entry and fault handling

- Every CPU entry path uses the frame shape required by its vector.
- Error-code and no-error-code exceptions enter a normalized dispatcher.
- A Ring 3 fault can terminate only the exact current process instance.
- An unhandled Ring 0 fault emits a useful serial record and halts deliberately.
- An unexpected vector never returns through a bare catch-all `iretq`.

### Page protection

- The kernel enables and verifies the CPU features required by the permissions it advertises.
- Writable and executable authority never coexist for one application mapping.
- User stacks and writable user data are non-executable.
- Supervisor writes respect read-only mappings.
- User-copy code is the only ordinary path that may access user mappings once SMAP is active.

### Ownership and cleanup

- Every frame, handle, process slot, request, packet buffer, socket, and persistent mutation has one owner at a time.
- Failed construction returns to the exact pre-operation state.
- A stale PID, slot, generation, handle, request ID, task ID, or completion cannot regain authority.
- Close, exit, fault, kill, cancellation, timeout, and device reset converge on defined cleanup paths.

### Bounded work

- Interrupt handlers do not block or perform unbounded loops.
- Parsers reject unbounded lengths before allocation or copy.
- Kernel queues have explicit budgets and observable saturation behavior.
- Recovery polling has a fixed budget and cannot become the hidden normal path.

### Evidence

- Every public guarantee has a positive test and at least one negative test.
- Every fixed crash or corruption case leaves a permanent regression input.
- A benchmark result identifies its exact commit, build, hardware, workload, and measurement method.

## Foundation workstream evidence

| Workstream | Required implementation evidence | Required negative evidence |
| --- | --- | --- |
| F0 verification | all supported targets build; workspace tests and QEMU matrix run | a removed marker or broken target makes CI fail |
| F1 traps | normalized frame and vector-specific entry | malformed return, user `#UD`, user `#GP`, and kernel fault tests |
| F2 protection | NX, write-protect, SMEP, and SMAP state recorded | execute-from-data, write-read-only, and supervisor-user execution tests |
| F3 memory | complete page-state accounting and rollback | OOM at each construction step, double free, reserved free, and >256 reclamation |
| F4 architecture | documented owner and interface for each mutable subsystem | dependency check rejects presentation-to-runtime mutation |
| F5 concurrency | explicit critical sections and per-CPU preparation | delayed, nested, and adversarial interrupt sequencing |
| F6 test modes | release and validation boot policies | release build cannot accidentally depend on a stress probe |
| F7 delivery | focused commits, ADRs, migration and rollback notes | PR check rejects missing evidence for affected risk classes |

## Verification architecture

GenOS uses several layers because no one test style can prove an operating system.

### 1. Host unit tests

Use host tests for pure logic and deterministic state machines:

- ELF and boot-contract validation;
- capability encoding, lookup, rights, generation, and revocation;
- allocator metadata and ownership transitions;
- filesystem snapshot decoding, validation, repair, and transaction selection;
- Ethernet, IPv4, UDP, TCP, DHCP, DNS, and VirtIO descriptor validation;
- scheduler policy and queue accounting;
- overflow, truncation, duplicate, and stale-identity behavior.

Host tests must avoid replacing the real hardware boundary with a mock that proves only the mock. The QEMU layer still validates entry, paging, interrupts, DMA, and device behavior.

### 2. Property and fuzz tests

Maintain independent fuzz targets for:

- boot information and memory maps;
- ELF headers, program headers, offsets, permissions, and segment overlap;
- partition tables and persistent filesystem generations;
- directory and path operations;
- Ethernet, ARP, IPv4, ICMP, UDP, DHCP, DNS, TCP, and VirtIO descriptors;
- syscall argument combinations and typed-handle confusion;
- serialized package and update metadata when those formats exist.

Each target must define a maximum input size and a meaningful invariant. “Did not panic” is useful but insufficient when the parser may silently accept malformed authority or mutate state.

### 3. QEMU system tests

The pull-request lane should cover at least:

- normal serial boot without a deterministic network server;
- deterministic network boot;
- no-network-device boot;
- first persistent commit and clean restore;
- torn-generation recovery;
- corrupted-storage failure;
- explicit read-only recovery;
- Ring 3 fault containment;
- process launch, exit, fault, kill, wait, and reclamation;
- capability forgery, wrong type, wrong rights, stale generation, and replay denial;
- allocation failure at selected boot-critical boundaries.

Every phase uses exact serial markers and a timeout. A fallback path must not emit the success marker for the preferred path.

### 4. Fault-injection tests

Fault injection should be deterministic and addressable by seed or operation number.

Required classes:

- allocation failure before and after each mutation;
- partial and failed block reads and writes;
- power loss between persistent-generation steps;
- packet loss, duplication, delay, reordering, truncation, checksum corruption, reset, and zero window;
- VirtIO timeout, malformed used index, invalid descriptor length, and device reset;
- cancellation during process, VFS, UDP, TCP, and accepted-stream work;
- interrupt delay and nested interrupt sequencing;
- copy-in or copy-out failure after authority validation but before completion.

A failed operation must expose an application-visible error when possible and preserve ownership invariants.

### 5. Long-run and repetition tests

The scheduled lane should include:

- at least 1,000 reference-VM boots;
- process creation and reclamation across PID and generation reuse;
- repeated mount, mutation, commit, restore, and repair cycles;
- sustained bounded network concurrency and injected loss;
- memory-pressure churn with allocator consistency checks;
- device reset and recovery loops;
- later, suspend and resume loops on reference hardware.

The test must report the number of completed iterations, first failing seed, resource counters before and after, and serial artifact.

### 6. Reference hardware tests

A physical-machine result includes:

- vendor and model;
- CPU, firmware version and mode, memory, storage, USB controller, network controller, and GPU;
- exact GenOS commit and image hash;
- boot arguments and enabled fallbacks;
- device discovery report;
- fault, reset, suspend, resume, and recovery outcomes;
- known unsupported hardware.

“Works on my machine” without this report is not an acceptance criterion.

## CI lanes

### Pull-request lane

Required before merge:

1. formatting;
2. Clippy with warnings denied for every shipped Rust target and profile;
3. workspace tests;
4. debug and release image construction where applicable;
5. fast QEMU matrix;
6. changed-parser fuzz smoke with retained corpus;
7. documentation link and format checks;
8. unsafe inventory and architecture-boundary checks;
9. changed benchmark sanity checks without enforcing noisy micro-optimizations.

### Main lane

Runs the pull-request lane from the merged commit and publishes artifacts:

- boot image and hash;
- serial logs;
- test summary;
- binary and image sizes;
- benchmark metadata;
- generated unsafe inventory;
- known-limitations snapshot.

### Scheduled lane

Runs longer work:

- repeated boots and lifecycle churn;
- extended fuzzing;
- full network fault matrix;
- storage power-loss matrix;
- performance regression suite;
- real-hardware jobs when infrastructure exists.

A scheduled failure creates a tracked issue or blocks the next release. It must not remain an unread notification.

## Test and release boot separation

Deep boot probes are valuable, but they should not define normal startup.

The project should expose explicit policies such as:

- `validation`: enables stress probes, fault injection, exact marker assertions, and deterministic host orchestration;
- `release`: enables only cheap invariants and user-visible diagnostics;
- `recovery`: enables conservative storage and hardware policy while denying mutation where required.

The selected policy must appear in the serial boot record. A release build must not claim success because validation-only code prepared hidden state for it.

## Unsafe-code discipline

Every `unsafe` block must have a nearby safety comment that states:

1. the memory, CPU, device, or aliasing invariant;
2. who established it;
3. how long it remains valid;
4. what synchronization or interrupt state protects it;
5. what input was validated;
6. how failure is contained.

The project should generate an inventory containing file, line, enclosing function, subsystem, and reason. A pull request that changes an unsafe block requires an explicit reviewer checklist entry.

Inline assembly receives the same treatment. Register clobbers, stack shape, privilege transition, interrupt state, and return behavior are part of the documented contract.

## Architecture and module health

Architecture quality is measured through ownership and change isolation, not by pursuing the smallest possible file count.

Track:

- largest source files and why they remain large;
- mutable globals and their owner or synchronization mechanism;
- unsafe blocks by subsystem;
- public ABI surface and compatibility version;
- dependency directions between architecture, memory, process, runtime, VFS, storage, network, driver, and presentation layers;
- number of unrelated subsystems changed by a typical feature;
- test and documentation coverage for public contracts.

A module should split when it owns unrelated state, changes for unrelated reasons, or prevents a subsystem from being tested independently. Mechanical splitting must not hide behavior changes.

## Pull-request scope

A good kernel pull request:

- solves one problem or establishes one contract;
- keeps each commit buildable and reviewable;
- separates mechanical movement from behavior;
- includes positive, negative, cleanup, and resource-exhaustion tests;
- documents new unsafe assumptions;
- updates the roadmap, limitations, subsystem contract, or ADR when the public understanding changes;
- explains rollback and compatibility;
- contains no unrelated cleanup.

Roughly 500 changed non-generated lines is a reviewability signal, not a hard law. Larger changes must explain why they cannot be split without creating an invalid intermediate state.

## Architecture decision records

Use an ADR when a change establishes or replaces a durable contract such as:

- syscall or application ABI;
- process and capability model;
- exception-frame layout;
- allocator policy;
- scheduler and concurrency model;
- storage format and transaction protocol;
- network queue or socket semantics;
- driver boundary;
- package, update, trust, or compatibility policy.

An ADR records context, decision, alternatives, consequences, migration, rollback, and verification. The template lives in [`docs/adr/0000-template.md`](adr/0000-template.md).

## Performance evidence

A performance change includes:

- hypothesis and metric;
- exact before and after commits;
- exact compiler, profile, CPU, firmware, QEMU, and device configuration;
- workload source and command;
- warm-up and cache policy;
- sample count, raw samples, median or other chosen summary, spread, and failures;
- CPU, memory, I/O, and queue context needed to explain the result;
- correctness tests run against both builds.

Do not optimize a benchmark by removing required work or comparing different feature sets without saying so.

Initial tracked metrics:

- firmware entry to usable serial prompt;
- kernel and boot image size;
- idle resident memory and allocated-frame count;
- idle wakeups and CPU time;
- syscall round trip;
- process launch and teardown;
- context-switch pair cycles;
- storage commit, restore, and recovery;
- UDP and TCP latency, throughput, loss recovery, CPU cost, and queue occupancy;
- later, input-to-frame and compositor latency.

## Comparison with Linux, Windows, macOS, BSD, or another OS

A comparison is an experiment, not branding.

The report must state:

- exact OS and kernel versions;
- enabled services and configuration;
- hardware or VM equivalence;
- feature equivalence and known omissions;
- workload and harness;
- result, variance, failures, and limitations;
- reproduction steps and raw data.

The title should name the measured result, for example “GenOS 0.xx reaches the reference serial shell with X MiB less memory than Linux Y under configuration Z.” It should not say “GenOS is faster than Linux” unless every relevant scope qualifier appears beside the claim.

Linux remains a valuable engineering reference for hardware breadth, filesystems, networking, scheduling, observability, security, and review process. GenOS should learn from those strengths while testing whether a smaller, integrated design can win on selected dimensions.

## Definition of done

A kernel or system change is done when:

- the documented success path works;
- malformed input and denied authority fail closed;
- timeout, cancellation, OOM, close, exit, fault, reset, and rollback behavior are defined where relevant;
- allocated resources return to the expected owner or free state;
- tests exercise real boundaries at the appropriate layer;
- CI is green;
- user-visible and developer-visible contracts are updated;
- performance and security claims include evidence;
- remaining limitations are recorded explicitly.

A roadmap checkbox, serial marker, or successful screenshot alone does not satisfy this definition.