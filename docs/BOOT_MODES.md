# Normal and validation boot

GenOS has two explicit startup policies. `validation-boot` is an opt-in Cargo
feature of both `kernel` and `genos-shell`; neither package enables it by default.
The kernel prints `BOOT_MODE normal` or `BOOT_MODE validation` immediately after
serial initialization. Compiler optimization level does not select the policy.

| Command | Kernel profile | Boot policy |
| --- | --- | --- |
| `cargo xtask build` | Development | Normal |
| `cargo xtask build-release` | Release | Normal |
| `cargo xtask run` | Release | Normal |
| `cargo xtask build-test` | Development | Validation |
| `cargo xtask test-release` | Release | Normal boot acceptance |

The image builder selects the matching shell feature for the policy. Building
only one package with `validation-boot` does not create a supported validation
image: its kernel and shell proof requirements would disagree.

## Normal startup

Normal boot claims the boot CPU before serial or global initialization, checks
the boot contract, establishes CPU/page protections and the protected IDT,
initializes memory ownership and networking, mounts the filesystems, and launches
the supervised Ring 3 shell. The shell checks its ABI and image-layout contract,
writes its banner through the real console capability, and waits for keyboard
input through the existing syscall and scheduler path. The serial terminal emits
`NORMAL_SHELL_READY` only after that input wait is live, then prints `genos>`.
`GENOS_READY` describes kernel initialization and is not sufficient proof of an
interactive shell.

Registering `INIT.ELF` is separate from executing it. Registration validates the
bounded ELF container and retains its immutable initrd slice. The usual process
constructor still validates mappings, permissions, and ownership on each launch.
A normal boot does not execute the init probe or mark it passed; the shell's
`run init` command remains available through the same supervisor capability.

Normal boot does not run lifecycle, supervisor cleanup, transaction rollback,
process-generation stress, scheduler benchmarks, the headless command transcript,
or the shell's filesystem, namespace, history, process-control, socket, and
protocol self-tests. In particular, startup does not create `/USER/SHELL.TXT` or
`/USER/APP.TXT` as proof fixtures. Normal filesystem mount/recovery behavior is
unchanged. Shell commands still perform their requested real operations.

## Validation startup

Validation images run the existing kernel and shell proof suites and retain their
exact success/failure markers. The kernel's `memory-test-faults` and
`network-test-faults` features imply `validation-boot`; the image builder must also
select the validation shell. The optional `SDK.ELF` application probe executes
only in validation mode. These checks deliberately launch and terminate test
processes, exercise temporary and persistent writes, send network requests, and
run stress and rollback cases.

Validation-only console marker recognition is compiled out of normal kernels.
A normal shell banner therefore cannot imply that namespace, history, networking,
or process-control proof suites ran. `probe_passed()` reports actual init-probe
completion; the retained graphical recovery status displays `not-run` for normal
boot.

Both policies retain the same privilege boundary, runtime ownership checks,
recovery decisions, public ABI, and real shell implementation. Separation does
not establish production readiness or complete the remaining fuzzing and repeated
boot requirements in [Roadmap F6](../ROADMAP.md#f6--test-boot-release-boot-fuzzing-and-fault-injection).

## Build compatibility and acceptance evidence

The supported toolchain is the pinned Rust 1.97.0. Release builds retain
optimization and one codegen unit with cross-crate LTO disabled. The precompiled
`x86_64-unknown-none` core library and GenOS's explicit large code model have
incompatible LLVM module flags under LTO; target checking alone did not expose
that link failure. `test-release` builds and boots both actual images.

The acceptance harness checks exact ordered readiness and console responses,
rejects validation markers in the normal kernel binary and serial output, and
uses fresh disposable test volumes. It verifies the absence of the two startup
fixture files, then executes `uname`, process launch/status/kill/reap, persistent
write, and read through the real Ring 3 shell. The development case has no NIC;
the release case uses modern VirtIO and requires `net` to report configured
networking, while the no-NIC case requires `network unavailable`. The dedicated
validation network suite retains the IPv6 echo proof. Logs, QEMU errors, image hashes, source status, tool versions and
commands are retained in `build/serial-normal-debug.log`, `build/serial-release.log`
and `build/normal-*-manifest.txt`. An incomplete manifest never proves a pass.
Host parser tests reject missing, duplicate, embedded and out-of-order evidence.
Each response must follow its own command echo, so launch-time process status
cannot satisfy a later `ps` command.
Each interactive boot has a 120-second wall-clock limit for cross-architecture
TCG and loaded hosts; passing still requires every ordered command response.

`make bench` explicitly selects validation policy for its scheduler probes and
restores a normal image even when the benchmark fails. Its timing remains a
validation-boot measurement and must not be presented as normal startup latency.

## Repeated normal boots

`cargo xtask test-repeat [COUNT]` builds the optimized normal image once and then
boots it repeatedly, with a freshly initialized disposable volume each time.
The default is 10; valid counts are 1 through 1000. Every iteration exercises the
same ordered shell, process launch/status/kill/reap, file write/read and network
diagnostics as `test-release`. It stops at the first failed iteration. A final
`NORMAL_BOOT_REPETITION_OK completed=N requested=N` line appears only after all N
iterations pass. Per-iteration logs and manifests use `repeat-0001` and subsequent
indices. One disposable data file is reused, so disk usage does not grow by 8 MiB
per boot. Copy evidence before starting another run, which reuses these names.

The separate `Long validation` workflow schedules 1000 boots weekly and supports
a manually selected count. It also runs three million-input parser campaigns in
independent jobs. Successful and failed runs retain artifacts. Adding this lane
does not claim its 1000-boot gate has already passed; the gate remains open until
a completed run supplies that evidence. Fresh boots do not replace sustained
single-boot memory-pressure or lifecycle churn testing.


## Interactive output policy

Normal kernels omit per-keystroke, syscall, lifecycle and successful commit trace
output. Fatal faults, startup configuration, storage failures and actual application
output remain visible. Validation builds retain the exact development event trace
through `serial::trace`, along with their probe suites. The large source diff for
this change routes existing diagnostic calls through that one policy; it does not
replace syscall handlers or change their authority checks.

The serial renderer echoes typed commands once, displays application output, and
handles `clear` with ANSI screen-clear/home sequences. Normal acceptance matches
the same plain command echoes and results a person sees, rejects leaked development
trace lines, checks `mem` and read-only diagnostic authority, rejects a dot-component
path, and proves clearing followed by another working command. It no longer depends
on hidden `USER_CONSOLE_WRITE` or process-event logging to declare success.

Help is split into ABI-bounded lines with a compile-time size check. The normal
acceptance sequence also samples `mem` before launch, while the example job is
live, and after kill/reap; usage must rise and return to the same baseline.
See [terminal commands](TERMINAL.md) for the current interactive scope.
