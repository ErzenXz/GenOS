# Process floating-point and SIMD state

GenOS uses an eager, fixed-size FXSAVE64 policy on its single-BSP reference
configuration. It preserves x87 control/status, tag and opcode fields, x87
instruction/data pointers, the eight x87/MMX register slots, all sixteen XMM
registers and MXCSR. This is the bounded legacy-state portion of roadmap F1.2,
not a claim that every processor feature or nested exception path is supported.

## Admission and enabled state

After the irreversible BSP/table-init claim, and before publishing the IDT,
`arch::init` requires CPUID leaf 1's x87, MMX, FXSR, SSE and SSE2 feature bits.
Missing support halts boot. It clears CR0.EM and CR0.TS, sets CR0.MP and CR0.NE,
and sets CR4.OSFXSR and CR4.OSXMMEXCPT. It clears CR4.OSXSAVE, so applications
cannot use AVX/AVX-512/AMX or other XSAVE-only state. It also clears EFER.FFXSR:
AMD's optional fast-FXSAVE mode must not omit XMM state from CPL0 saves.
Control-register/MSR readback must match before `CPU_XSTATE_READY` is emitted.

The fixed policy is deliberate: current programs do not need variable-sized
XSAVE components, and 512 bytes per process is a bounded memory cost. Supporting
additional state requires a separate CPUID/XCR0/size/alignment policy and matching
isolation tests. Exposing an instruction-set feature in hardware CPUID alone is
not an OS promise that its extended state is enabled.

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

Existing fatal behavior is unchanged: a kernel-origin fault halts, and supported
process-local faults terminate only the current process. Same-IST nesting,
return-instruction faults, emergency-stack guards and NMI recovery remain
separate unfinished work. This policy also does not establish TLS/FS/GS/debug
register virtualization, SMP state ownership, or physical-CPU qualification.

## Validation

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
executes `UD2` while its private state is live; the other two exit normally.

The kernel compares all architectural state fields in those user-written
snapshots and the final saved image, checks completion and yield/preemption
counts, reclaims all three address spaces, and requires the original frame
baseline. It repeats with the same identities and new patterns. All six initial
snapshots must contain the prescribed clean state, including x87 payloads.
The exact marker is emitted only after both rounds and cleanup succeed:

```text
USER_XSTATE_OK processes=6 rounds=2 components=x87,mmx,xmm0-15,mxcsr syscalls=direct,yield faults=2 fresh=6 reclaimed=true
USER_XSTATE_PREEMPTIONS count=12
```

The count may exceed twelve. This fixture exists only in validation images;
normal images contain the preservation mechanism without running the probe.
The isolated exception harness explicitly omits this additional fixture in its
disposable source copy, preserving its assertion of exactly one selected fault.
The full validation harness requires the xstate markers. Passing this bounded
QEMU case does not prove every physical CPU, pending/unmasked FP exception,
instruction-set combination or interrupt nesting pattern.
