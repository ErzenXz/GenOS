# Kernel stack guard contract

Each statically reserved kernel stack has its own inaccessible 4 KiB page on
both sides of its usable storage. The stack capacities remain unchanged:

| Stack | Usable bytes | Entry owner |
| --- | ---: | --- |
| Boot/runtime | 2 MiB | Admitted BSP, then kernel coordinator |
| Ordinary IRQ/exception | 64 KiB | IST 1 |
| User privilege transition | 64 KiB | TSS RSP0, including `int 0x80` |
| Double fault | 16 KiB | IST 2 |
| NMI | 16 KiB | IST 3 |
| Machine check | 16 KiB | IST 4 |
| Debug exception | 16 KiB | IST 5 |

`GuardedStack<N>` has a C layout and page alignment: lower guard, exactly N
usable bytes, upper guard. Every allocation includes both guards, and adjacent
stacks share no guard or usable page. The empty stack pointer equals the upper
guard's starting address; a push writes below it. TSS entries and the initial
boot-stack transfer use this adjusted top. The guard storage remains reserved
inside the kernel ELF and is never returned to the physical allocator.

## Installation and containment

The admitted BSP first installs its GDT/TSS/IDT, creates the supervisor page-table
clone and applies kernel image permissions. Image protection splits the BSS
mapping into 4 KiB leaves. With IF still clear, guard installation verifies all
seven layouts are disjoint and the live RSP lies inside the boot stack. For each
of the fourteen guard pages it then:

1. Requires the protected kernel root to be active and the address to belong to
   the kernel data image.
2. Walks existing supervisor tables, rejecting missing or huge mappings.
3. Requires an identity-mapped, supervisor-only, non-executable 4 KiB leaf.
4. Clears that leaf, invalidates the BSP translation with INVLPG, and verifies
   that the address no longer translates.

This operation allocates no page tables, frees no backing frames, and changes
no neighboring mapping. Failure halts boot before IRQ enable. A partially
installed set is never advertised as ready. Successful installation emits:

```text
KERNEL_STACK_GUARDS_READY stacks=7 guards=14 bytes=4096
```

Only then are the final IRQ/syscall gates installed, the IDT made read-only and
hardware interrupts enabled. Later process roots share the already guarded
supervisor mappings. No normal operation remaps these holes.

An access to a guard raises a kernel page fault. The existing normalized fault
entry reports its frame and CR2 and deliberately halts; it does not resume an
overflowed kernel stack. IST supplies the exception stack independently of the
faulting RSP. The IRQ-floor probe specifically exercises a fault with RSP at the
ordinary IST stack's lower boundary, where entry reloads that stack's configured
top before the fatal handler runs. This tests a bounded fatal path, not safe
recovery of arbitrary nested frames.

## Verification and remaining limits

The host geometry tests check storage offsets, alignment, preserved usable
capacities, pointer boundaries, overflow rejection and disjoint adjacent stacks:

```sh
cargo test -p kernel --lib stack::tests
python3 -m unittest discover -s tools -p test_stack_guard_harness.py
python3 tools/test_stack_guards.py
```

The VM harness requires clean committed source and creates disposable source
copies. Each copy injects one deliberate fault after production guard and IDT
installation. Three cases move RSP to the boot, privilege or IRQ stack floor
and execute a real PUSH into the lower guard. The fourth writes the double-fault
stack's upper guard from the intact boot stack. The harness checks every stack's
two-guard geometry, exactly one supervisor non-present write page fault, its
exact CR2, the expected faulting RSP for PUSH, and the explicit fatal halt. It
rejects missing/duplicate/wrong-phase evidence, successful return, continued boot
or QEMU exit/reset. It retains source identity, fixture diff, image hash, command,
serial output and success/failure manifests under `build/stack-guard-evidence`.

All four CPU cases passed in the September 22 campaign recorded in
[VERIFICATION.md](VERIFICATION.md). The parser's negative tests supplement those
real CPU cases.
NMI, machine-check and debug guards receive installation/readback and geometry
checks; this bounded campaign does not individually overflow those handlers.

Guards become active only after protected page tables and image permissions are
installed. The earlier firmware/admission and initial boot-stack setup therefore
remain outside their protection window. A large unchecked stack-pointer jump
can skip a single-page guard; compiler stack probing and stack-use budgeting
remain necessary. Per-stack high-water measurements, repeated NMI/same-IST
nesting, fault-during-return, real double-fault recovery and physical-hardware
qualification remain open. The guards complete a containment slice of roadmap
F1.3, not the entire stack or nested-exception gate.
