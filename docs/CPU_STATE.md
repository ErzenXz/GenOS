# Bounded process CPU state

GenOS uses an eager, fixed-size FXSAVE64 policy on its single-BSP reference
configuration. It preserves x87 control/status, tag and opcode fields, x87
instruction/data pointers, the eight x87/MMX register slots, all sixteen XMM
registers and MXCSR. Private DS/ES/FS/GS selectors accompany that image. This is
the bounded reference-profile portion of roadmap F1.2,
not a claim that every processor feature or nested exception path is supported.

## Admission and enabled state

After the irreversible BSP/table-init claim, and before publishing the IDT,
`arch::init` requires CPUID leaf 1's x87, MMX, FXSR, SSE and SSE2 feature bits.
Missing support halts boot. It clears CR0.EM and CR0.TS, sets CR0.MP and CR0.NE,
and sets CR4.OSFXSR and CR4.OSXMMEXCPT. It clears CR4.OSXSAVE, so applications
cannot use AVX/AVX-512/AMX or other XSAVE-only state. It also clears EFER.FFXSR:
AMD's optional fast-FXSAVE mode must not omit XMM state from CPL0 saves.
CR4.PKE is cleared separately, because user PKRU instructions can otherwise
expose unsaved state independently of OSXSAVE.
CR4.FSGSBASE is also cleared: direct user FS/GS base writes are unavailable until
the kernel has a complete TLS/context contract. This opt-in is independent of
OSXSAVE, as described in [Intel's FSGSBASE guidance](https://www.intel.com/content/www/us/en/developer/articles/technical/software-security-guidance/best-practices/guidance-enabling-fsgsbase.html).
Control-register/MSR readback must match before `CPU_XSTATE_READY` is emitted.
EFER.SCE is cleared as well: `int 0x80` is the only owned application entry.
On SEP-capable CPUs, IA32_SYSENTER_CS/ESP/EIP are zeroed and read back; CPUs
without SEP cannot execute SYSENTER. Unowned firmware entry points must never
remain callable by applications. Intel's 64-bit SYSENTER with a null entry
selector raises #GP; AMD defines SYSENTER in long mode as #UD. The hardware
fixture names its Intel vendor variant explicitly rather than conflating them.

The fixed policy is deliberate: current programs do not need variable-sized
XSAVE components, and 512 bytes per process is a bounded memory cost. Supporting
additional state requires a separate CPUID/XCR0/size/alignment policy and matching
isolation tests. Exposing an instruction-set feature in hardware CPUID alone is
not an OS promise that its extended state is enabled.
The [CPU feature matrix](CPU_FEATURES.md) states the exact admission and disabled
instruction contracts and the independent hardware variants used to test them.

Each newly constructed process owns a private, 16-byte-aligned 512-byte image
inside its kernel process record. The image begins with FCW `0x037f`, MXCSR
`0x1f80`, an empty x87 tag word, and zeroed register payloads, pointers and other
fields. Construction always replaces the image, including when process storage
or a PID is reused. User memory cannot modify the authoritative image.

The layout and instructions follow the [Intel architecture manuals](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html).
The pinned [`x86_64-unknown-none` target](https://doc.rust-lang.org/rustc/platform-support/x86_64-unknown-none.html)
generates no floating-point or vector-register operations by default. A kernel
compile-time guard rejects SSE/AVX target features. Kernel Rust must retain this
soft-float policy; enabling SIMD in a function or handwritten kernel assembly
outside the state-management boundary would violate the contract. The explicit
MMX instructions in the validation fixture execute only in Ring 3.

## Entry, return and ownership

`UserContext` keeps its existing twenty 64-bit words: fifteen saved GPRs followed
by RIP, CS, RFLAGS, RSP and SS. Syscalls and the timer use that same frame; generic
exceptions insert vector and error words before the architectural IRET frame.
Assembly obtains the CS offsets from Rust `offset_of!` constants.

`run_slice` disables local interrupts, publishes the current process and switches
CR3. Its assembly entry receives the context in RDI and the aligned state image
in RSI, publishes that image's address, restores it with FXSAVE's 64-bit restore
form, then loads the GPR/IRET frame and enters Ring 3. The process object cannot
move while this call is active, and its supervisor mapping remains present in
both address spaces.

Every Ring 3 IRQ, exception and syscall entry captures the image with FXSAVE64
after saving GPRs and before calling Rust. IRQ/exception entry checks the saved
CS so an interrupt of kernel work never overwrites a user's image. The helpers
clobber only RAX, which is already saved; they preserve the remaining GPRs and
flags. FXSAVE does not wait for pending x87 exceptions. The interrupt gate leaves
IF clear, and DF/AC are cleared before Rust as before.

A direct IRET restores the same image before restoring GPRs. A scheduling,
exit or fault path instead consumes the captured image and returns through the
kernel trampoline; it does not overwrite the capture after Rust has run. The
trampoline clears the published image pointer before returning to the scheduler.
No floating-point instructions run in kernel Rust while user state is live.
There is no lazy `#NM` ownership or shared process save buffer.

The same assembly helpers capture four visible segment selectors in a private
eight-byte `segments::State`, replace them with kernel DS/ES `0x10` and null
FS/GS before Rust, and restore the process selectors on each direct return or
new slice. Fresh processes start with DS/ES `0x2b`, FS/GS null. The initialized
GDT has only flat user data/readable code descriptors, LDTR is explicitly null,
and FSGSBASE and arbitrary MSR writes are unavailable to users. Boot zeros and
reads back FS_BASE, GS_BASE and KERNEL_GS_BASE. Descriptor loads can only install
zero bases; loading a null selector can retain only an already-zero base. This
bounded policy needs no SWAPGS or TLS API. A future nonzero base interface must
save bases separately from selectors and revisit kernel entry/NMI ordering.

Boot disables all hardware breakpoints, zeros DR0–3 and clears DR6 status, with
DR7/DR6/address readback before application admission. No API grants debug-register
authority. DR6 may subsequently acquire exception status, but applications cannot
read it and the current fault handler does not use it to grant recovery. User
single stepping is a process-local #DB termination, not a debugger service.

### Reference register inventory

| State visible to a 64-bit application | Current ownership and boundary |
| --- | --- |
| Fifteen GPRs, RIP/RSP, CS/SS, RFLAGS | Private `UserContext`; CPU frame captures privilege/flags; direct and scheduled return restore it |
| DS/ES/FS/GS selectors | Private eight-byte selector image; captured before Rust, kernel selectors while Rust runs, restored before user return |
| FS/GS hidden bases | Fixed zero; flat GDT, null LDT, disabled FSGSBASE, no base-setting syscall; boot clears inherited MSRs |
| x87/MMX, XMM0–15, MXCSR | Private aligned 512-byte FXSAVE64 image; eager entry capture/return restore, including pending x87 exception state |
| Debug addresses/control | Kernel-owned disabled policy; privileged MOV DR; #DB terminates the current process; no debugger virtualization |
| XSAVE-only state, PKRU | OSXSAVE/PKE disabled; unsupported instructions fault; no extended-state interface |
| CRs, descriptor tables, TSS, MSRs, I/O privilege | Kernel-owned; CPL3 writes denied; GDT/TSS admit only 64-bit user code and deny port I/O; only `int 0x80` provides a syscall entry |
| CPUID, timestamp observations | Machine-wide observations, not private saved registers; no isolation or virtual-time promise |

This inventory covers the admitted reference feature set. Larger CPU profiles,
performance-monitoring access, future CET/UINTR/extended state, exhaustive flag
interactions and other machine-wide observations need their own policy/evidence.

Kernel-origin faults halt, and supported process-local faults terminate only
the current process. The [entry ownership guard](FAULT_CONTAINMENT.md) now
halts nested entries and #DF in assembly before Rust or CPU-state helpers.
Native return flags are validated before admission and on syscall/timer capture.
Kernel stacks have [dedicated guard pages](KERNEL_STACKS.md). This policy does not
provide nonzero TLS bases, debugger APIs, SMP state ownership, or physical-CPU
qualification. The internal selector image changes no public ABI or file format;
rollback uses a matched older kernel/userspace bundle. See
[ADR-0013](adr/0013-bounded-user-cpu-context.md).

## Validation

### Five native integer-state tasks

The validation-only integer ELF adds five bounded F1.2 evidence tasks through
the unchanged loader, mapper, constructor, syscall and timer paths:

| Task | Required observation |
| --- | --- |
| Fresh state | Six Ring 3 snapshots: fourteen zero GPRs, only the declared RDI token, initial RSP, exact `0x202` flags and native CS/SS |
| Direct syscall | All fifteen distinct nonzero GPR patterns before an unknown syscall; fourteen preserved and RAX replaced by the exact ABI error, with exact RSP/flags/selectors |
| Yield | All fifteen GPRs across a real scheduled yield, including the six zero arguments required by the existing ABI and RAX's zero result |
| Timer | Two actual PIT captures per process inside a flag-neutral MOV/LEA/JRCXZ loop; all GPRs, RSP, flags and selectors checked before progress is published |
| Fault and reuse | Two real user #UD terminations, four healthy peer exits, unconditional address-space reclamation and exact frame baseline; repeat the same identities/slots with new patterns and clean initial state |

Three simultaneously live processes run twice. Each process has a distinct value
in every GPR; RBX points to a distinct offset in its private progress page.
RCX is the timer loop's sole progress scratch: kernel captures admit only the
MOV/LEA intermediate values `0, 1, 2, -1, -2`; all other saved words compare
exactly. The before-loop snapshot requires `-2` and after-loop snapshot requires
zero. Captures must have RIP inside the linked loop, so unrelated earlier
preemptions cannot meet the two-capture requirement. The bounded coordinator
allows at most 48 rounds and reclaims every constructed owner on failure.

The patterns include arithmetic flags, DF, AC and ID, with different combinations
in each round. All memory/stack accesses are aligned. User PUSHFQ observations
and saved timer frames compare every flag bit, without masking DF/AC. The
existing entry assembly clears live kernel DF/AC before Rust; this fixture
qualifies private user preservation and successful entry/return, not every
alignment exception, single-step/RF interaction or independently sampled kernel
flag at every instruction. Fault snapshots precede UD2; exception GPR frames
are not relabeled as resumable process contexts. No ABI or normal-image behavior
changes; this suite runs after the xstate suite and the isolated exception
harness omits both through the existing single probe entry.

Only after both rounds and cleanup succeed are these five exact markers emitted:

```text
USER_INTEGER_FRESH_OK processes=6 gprs=15 rsp=initial flags=0x202
USER_INTEGER_DIRECT_OK processes=6 preserved=14 rax=unknown-syscall flags=exact
USER_INTEGER_YIELD_OK processes=6 gprs=15 args=zero flags=exact
USER_INTEGER_TIMER_OK processes=6 captures=12 flags=arithmetic,df,ac,id
USER_INTEGER_REUSE_OK rounds=2 faults=2 vector=6 exits=4 reclaimed=true
```

The aggregate validation evidence contract requires them once, in order, between
CPU context and shell readiness. Normal images forbid them. Host tests flip
every bit in all fifteen GPRs, RSP, flags and selectors to prove the comparison
does not ignore a field. The independent harness additionally requires both
actual #UD records, six yields and four peer exits in the correct phase:

```sh
python3 tools/test_integer_context.py
python3 tools/test_integer_context.py --case gpr-corrupt
python3 tools/test_integer_context.py --case flags-corrupt
python3 tools/test_integer_context.py --case timer-skipped
```

The intact case builds an unchanged clean committed source archive. Three negative
cases change only a saved user observation or the user loop in disposable copies;
they require actual rejection, cleanup back to baseline and no positive marker.
All cases retain the original source archive, private patch, source/image and
environment identity, commands, serial output and failure manifests, and check
QEMU remains live before host cleanup. It stops after these CPU proofs, without
requiring external DNS. The existing Build and boot CI context runs it before
the full suite and retains its evidence. That wiring alone is not passing CI
or frozen-reference/physical qualification. F1.2 remains open.

### Existing floating-point and selector tasks

Host tests check required feature rejection, forbidden control-state modes,
initial image size/alignment/zeroing, and coverage of every defined restorable
byte while excluding hardware mask/reserved bytes:

```sh
cargo test -p kernel --lib xstate
```

Validation boot additionally loads a purpose-built ELF through the production
parser, mapper and process constructor. Three simultaneously live Ring 3
processes install different values in every x87 and XMM slot and different
rounding controls. Each observes at least two actual PIT preemptions, takes a
direct syscall, saves its own observed registers to its private data page,
writes all eight MMX aliases, yields, and snapshots again. One process then
raises user #MF with its private pending exception live; the other two exit normally.

Each process also installs a different combination of null, user data and
readable code selectors. User snapshots before/after direct and yielding
syscalls, plus the final kernel capture, must match. FS/GS-relative data reads
check the zero-base policy. The odd peer generates an unmasked x87 invalid
operation, carries its pending exception through a direct syscall, yield and
another real PIT preemption, then executes FWAIT to raise exactly user #MF.
The two pending snapshots and final kernel image must match all restorable
fields; the saved FCW/FSW must retain the unmasked invalid-operation state.
Subsequent reused peers must observe fresh masked state and initial selectors.

The kernel compares all architectural state fields in those user-written
snapshots and the final saved image, checks completion and yield/preemption
counts, reclaims all three address spaces, and requires the original frame
baseline. It repeats with the same identities and new patterns. All six initial
snapshots must contain the prescribed clean state, including x87 payloads.
The x87 phase checks all 80 bits of each register; the MMX phase checks its
64-bit payload and treats the high sixteen slot bits as reserved, as specified
by the [Intel FXSAVE instruction table](https://cdrdv2-public.intel.com/868140/253666-089-sdm-vol-2a.pdf).
The two phases use different payloads. A regression test ensures the MMX rule
does not weaken the x87 comparison or ignore any MMX payload bit.
The exact marker is emitted only after both rounds and cleanup succeed:

```text
USER_XSTATE_OK processes=6 rounds=2 components=x87,mmx,xmm0-15,mxcsr syscalls=direct,yield faults=2 fresh=6 reclaimed=true
USER_XSTATE_PREEMPTIONS count=14
USER_CPU_CONTEXT_OK processes=6 selectors=ds,es,fs,gs bases=zero pending_mf=2 fresh=6 reclaimed=true
```

The extended fixture and evidence parser require at least fourteen total preemptions.
This fixture exists only in validation images;
normal images contain the preservation mechanism without running the probe.
The isolated exception harness explicitly omits this additional fixture in its
disposable source copy, preserving its assertion of exactly one selected fault.
The full validation harness requires the xstate markers. Passing this bounded
QEMU case does not prove every physical CPU, every FP exception class,
instruction-set combination or interrupt nesting pattern. Independent real CPU
fixtures additionally check user #MF/#XM termination and reclamation, kernel
#MF/#XM deliberate halt, forbidden SYSCALL/Intel SYSENTER, and recovery from
explicitly poisoned inherited FS/GS bases and an enabled hardware breakpoint:

```sh
python3 tools/test_exception_entry.py --mode user --fault mf
python3 tools/test_exception_entry.py --mode user --fault xm
python3 tools/test_exception_entry.py --mode kernel --fault mf
python3 tools/test_exception_entry.py --mode kernel --fault xm
python3 tools/test_cpu_features.py --case isa-syscall
python3 tools/test_cpu_features.py --case isa-sysenter
python3 tools/test_cpu_features.py --case aux-inherited
```

Instruction and selector semantics were checked against the
[Intel manuals](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html),
[Intel FSGSBASE guidance](https://www.intel.com/content/www/us/en/developer/articles/technical/software-security-guidance/best-practices/guidance-enabling-fsgsbase.html)
and [AMD's system-programming manual](https://www.amd.com/content/dam/amd/en/documents/processor-tech-docs/programmer-references/24593.pdf).
Executed evidence and exact tested source belong in [VERIFICATION.md](VERIFICATION.md).

### SIMD exception qualification limit

The strict CI user/kernel #XM probes now select the explicit
[KVM CPU qualification variant](REFERENCE_VM.md#hardware-cpu-qualification-variant).
The same unmasked DIVSS fault instruction and vector-19 cleanup/halt validators
run with accelerator preflight and QMP readback. This supplements the TCG
reference; the retained emulation failures below remain part of its limitations.

On the pinned QEMU 11.1.1 TCG reference, the strict user and kernel #XM
fixtures **fail**: DIVSS 0/0 returns instead of trapping with the invalid-operation
mask cleared. A separate diagnostic ELF verifies MXCSR `0x1f00` immediately
before and `0x1f01` after the instruction; its disassembly verifies LDMXCSR,
STMXCSR, XORPS and DIVSS. Kernel CR4 readback already establishes OSXMMEXCPT.
The existing dispatch policy would terminate user vector 19 and halt kernel
vector 19, but these runs cannot prove delivery or containment of an actual #XM.

The strict fixtures, failed manifests and configured matrix jobs remain; a
returned probe or latched MXCSR flag is never accepted as exception success.
Both unchanged probes pass on the KVM CPU variant at `818a5b1`; complete
F1.2 qualification remains broader than these two cases. The frozen reference
and its failed evidence remain recorded. QEMU's [SSE implementation](https://github.com/qemu/qemu/blob/master/target/i386/ops_sse.h)
and [FP helpers](https://github.com/qemu/qemu/blob/master/target/i386/tcg/fpu_helper.c)
support the emulator-limit diagnosis, but those upstream files are not an exact
source attribution for the locally installed binary. The retained runtime
readbacks are the direct evidence for this host.
