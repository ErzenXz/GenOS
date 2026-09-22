# Kernel ELF admission and loading

The UEFI loader validates the complete load plan before reserving destination
pages or copying any segment. `bootloader::kernel_elf::Image` is a pure `no_std`
module; it borrows the immutable source bytes and has no firmware calls, heap
allocation, raw-pointer access or unsafe code. The small firmware adapter in
`bootloader/src/elf.rs` consumes that plan.

This fixes the previous loader's typed references into potentially unaligned
input, unchecked physical-range arithmetic, and discovery of later malformed
segments only after allocating or copying earlier ones. Malformed loading
metadata now returns `LOAD_ERROR` with `KERNEL_ELF_REJECTED reason=...` before
those side effects.

## Supported image contract

The format fields follow the [ELF header](https://gabi.xinuos.com/elf/02-eheader.html)
and [program loading](https://gabi.xinuos.com/elf/07-pheader.html) definitions.
The following limits and restrictions are GenOS policy, not universal ELF rules:

- ELF64, little-endian, current identification/header version, `ET_EXEC`,
  x86-64, unspecified/System V OS ABI with ABI version zero, zero architecture
  flags, a 64-byte ELF header and 56-byte program-header entries.
- One through 32 program headers, with at most 16 nonempty load segments. The
  complete program-header table must fit the file and start after the ELF header.
  Extended program-header counts are unsupported.
- The first load segment starts at the kernel linker's current 32 MiB address.
  Virtual and physical addresses must match. The complete page-rounded image,
  including holes and BSS, spans at most 64 MiB from that base. This cap bounds
  the contiguous reservation and initialization work; it does not preallocate
  64 MiB for a smaller kernel.
- Checked file intervals fit the input; file size never exceeds memory size.
  Memory ranges cannot wrap. Load segments appear in ascending address order
  and share no destination page, including partially occupied end pages.
- Load segments are readable, have no unknown permission bits, and never request
  write and execute together. The entry address falls within the initialized
  file-backed portion of an executable load segment, outside BSS and holes.
- File offset and load address agree modulo 4096. Declared alignment is zero,
  one, or a power of two; any alignment greater than one also requires matching
  file/address residues.

`PT_NULL` is ignored. Informational `PT_NOTE` data must have a bounded file range,
but its contents do not establish platform features or trust. At most one
`PT_GNU_STACK` entry is accepted, with RW/non-executable flags and no requested
address, file data, size or alignment; the kernel supplies its own stack.

The current linker emits `PT_GNU_RELRO`. At most one is accepted, with read-only
flags and a file/address mapping contained in an existing initialized load
segment. It is descriptive metadata here: the loader does not implement dynamic
relocation or apply extra RELRO page protection. Actual kernel text, rodata and
data permissions are installed later by the kernel's linked-section policy.
Rejecting W+X ELF requests is not proof of hardware W^X during the firmware stage
or across physical aliases.

Interpreter, dynamic-linking, TLS, `PT_PHDR` mapping, GNU property and other
unrecognized segment types are refused rather than silently promising unsupported
loader work. Section/debug/symbol tables are not used or validated. Reserved ELF
identification padding is ignored. This is a validator for the GenOS kernel's
loading contract, not a general ELF conformance checker or an authenticity check.

## Allocation, copying and collisions

The adapter verifies that the live source byte range does not intersect the
complete destination span, including padding and holes. It then requests one
exact-address UEFI allocation. Firmware remains responsible for refusing a
collision with existing loader code, stacks, input buffers, firmware regions
and other live allocations. The returned address is checked before any write;
an unexpected returned allocation is released without exposing it.

The immutable plan is complete before that allocation. After a successful grant,
there is no fallible parser operation: raw writes initialize the entire span to
zero, then the plan's bounded source slices are copied to their offsets. Raw
initialization avoids creating a Rust reference to uninitialized `u8` memory.
No destination reference or executable entry is published before copying ends.
The pages remain `LOADER_CODE` and are retained across the boot handoff.

These guarantees assume a conforming, identity-mapped x86-64 UEFI allocator and
accessible source bytes supplied by the filesystem layer. They do not contain
hostile firmware/DMA, authenticate the kernel, or recover from a machine fault
while writing newly allocated RAM. File reads and later initrd/handoff
construction are separate allocation boundaries; exhaustive failure injection
for those paths remains open under F1.1/F3.4.

## Verification

```sh
cargo test -p bootloader --lib
cargo clippy -p bootloader --lib -- -D warnings
cargo clippy -p bootloader --target x86_64-unknown-uefi -- -D warnings
cargo run -p bootloader --example check_kernel -- target/x86_64-unknown-none/debug/kernel
cargo run -p bootloader --example check_kernel -- target/x86_64-unknown-none/release/kernel
```

Fourteen host tests cover every truncated prefix of a representative image,
misaligned source/table storage, malformed header fields, bounded metadata,
file/memory overflow, entry/permission/alignment rejection, byte/page overlap,
source collision, unsupported loader requirements, and exact copy/BSS/padding
behavior. The copy test also proves that an incorrectly sized destination stays
unchanged. The `check_kernel` example validates and simulates loading an existing
kernel image; it does not build that image or execute firmware/CPU code.

The actual local debug and release kernel images were accepted with four load
segments each. The full boot/handoff suite and all six malformed-image VM cases
passed locally; see [the exact verification record](VERIFICATION.md). Broader
mutation, coverage-guided fuzzing and other firmware implementations remain
further qualification work.

`python3 tools/test_kernel_elf.py` replaces the staged kernel in a disposable
image with six malformed fixtures. Each case requires the exact ordered loader
rejection, no kernel entry, exit status zero, and a QMP `SHUTDOWN` event with
`guest: true` and `reason: guest-shutdown`. QEMU starts paused until QMP is
subscribed, so early terminal events are observed. Reset, panic, host exit,
missing events and expired deadlines fail the case. Raw QMP input and commands
are retained and hashed alongside serial output, including failed cases. QMP
connection, message sizes, message count and total input are bounded; a short
owned socket path supports the macOS reference host.
