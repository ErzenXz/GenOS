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

## Usage measurements and overflow bounds

Before its first call, naked BSP entry fills the usable boot stack with a fixed
word pattern. GDT/TSS initialization fills the six other usable stacks before
publishing their pointers. Guards are excluded. `stack_usage` scans usable words
with bounded assembly MOV reads while local IRQs are masked and reports the
deepest changed word's distance from the top. No Rust slice or typed memory read
is created over live frames, which can contain logically uninitialized padding
or moved locals despite the original fill. The assembly defines its register
output from the observed hardware bits without treating such bytes as a Rust
value.

Validation emits one `KERNEL_STACK_USAGE` record for each stack after its console
and process workload, including usable capacity, touched high-water bytes and
remaining bytes. Qualification fails if any measured remaining budget is below
4 KiB. The dedicated usage fixture exercises a known depth on all seven stacks
and verifies the measured depth, arithmetic and minimum remaining budget.

This pattern method measures touched storage to eight-byte granularity. It
includes measurement overhead and can undercount storage that was reserved but
not touched, or overwritten with the same pattern. It is therefore an engineering
measurement, not the protection mechanism or an exact maximum-RSP proof.

The protection policy also requires the pinned Rust target's generated stack
probes. A dedicated fixture enters a non-inlined Rust function with less than one
page left and a 12 KiB local array whose address escapes optimization. The first
fault must occur inside the guard, with RSP not below that guard; skipping the
guard into adjacent memory fails qualification. Handwritten stack adjustments
remain confined to the fixed entry frames, known stack-top transfers and this
explicit fixture. New dynamic stack allocation or unchecked manual RSP changes
require their own bounded contract and probe evidence.

## Verification and remaining limits

The host geometry tests check storage offsets, alignment, preserved usable
capacities, pointer boundaries, overflow rejection and disjoint adjacent stacks:

```sh
cargo test -p kernel --lib stack::tests
python3 -m unittest discover -s tools -p test_stack_guard_harness.py
python3 tools/test_stack_guards.py
```

The VM harness requires clean committed source and creates disposable source
copies. Fourteen cases each inject one deliberate fault after production guard
and IDT installation: a real PUSH at each stack floor and a write at each upper
guard. Two additional cases exercise usage measurements and the compiler's
multipage stack probing. The harness checks every stack's
two-guard geometry, exactly one supervisor non-present write page fault, its
exact CR2, the expected faulting RSP for PUSH, and the explicit fatal halt. It
rejects missing/duplicate/wrong-phase evidence, successful return, continued boot
or QEMU exit/reset. It retains source identity, fixture diff, image hash, command,
serial output and success/failure manifests under `build/stack-guard-evidence`.

The earlier four-case guard campaign passed as recorded in
[VERIFICATION.md](VERIFICATION.md); the expanded campaign must retain its own
exact-commit evidence before qualification. Harness negative tests cover missing
and forged measurements, incorrect fault addresses, missing margins and a
compiler-probe RSP that skipped below its guard.

Guards become active only after protected page tables and image permissions are
installed. The earlier firmware/admission and initial boot-stack setup therefore
remain outside their protection window. General unchecked stack-pointer jumps
are unsupported; the compiler probe and fixed assembly entry audit are required
parts of the reference policy. Repeated NMI/same-IST nesting, fault-during-return,
real double-fault recovery and physical-hardware qualification remain separate
work; ordinary guard containment does not establish them.
