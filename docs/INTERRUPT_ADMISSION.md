# Interrupt initialization admission

The admitted BSP initializes GDT/TSS/IDT once, then initializes the legacy
interrupt controllers once. `BootClaim` now publishes completion between those
phases. This narrows F4.2/F5.1; it does not classify every mutable object or
establish SMP, IRQ latency, device reset or complete interrupt qualification.

## Failure and ownership contract

The irreversible phases are unclaimed → boot claimed → architecture initializing
→ architecture ready → controllers initializing → controllers ready. Each
transition checks the hardware BSP role and expected phase before an atomic
compare/exchange. An AP, unsupported CPU, premature call or repeated transition
cannot consume the rightful next phase. A started setup has no reset/retry path;
interrupted or failed setup never becomes ready through ordinary admission.
Acquire/release publishes phase metadata; adapters remain responsible for the
hardware operations and their order. The pure gate is not a hardware verifier.

`arch::init` fills all 256 IDT entries before LIDT. The same pass selects timer,
keyboard, mouse, MSI-X and syscall handlers, granting user DPL only to the
syscall vector. The former later IDT setters are removed. GDT/TSS setup,
xstate/auxiliary state and machine-check policy complete before architecture
readiness is published. A second architecture call remains refused by the
original BSP table claim.

`interrupts::init` clears IF, claims the controller phase, then remaps the PIC
and programs the PIT. A refused call prints
`INTERRUPT_INIT_REJECTED policy=halt` before controller I/O. It performs no IDT
writes. Successful controller setup publishes readiness once.
`interrupts::enable` checks BSP/controller readiness before STI; refusal clears
IF and prints `INTERRUPT_ENABLE_REJECTED policy=halt`. The gate does not itself
prove that paging permissions or all devices are ready. Production main still
installs stack guards before controller setup and seals the IDT and supervisor
mappings before enabling interrupts. Raw CPL0 helpers retain their caller
obligations; this is not a complete safe API for arbitrary kernel code.

Initialization never waits for another claimant. Rejection halts with ordinary
interrupts masked; it promises no hardware rollback or recovery. CLI does not
mask NMI, machine check or synchronous exceptions. Existing emergency assembly
and nested-entry policy remain necessary. The TSS/emergency slots still have
assembly-owned runtime mutation and require the wider unsafe-boundary review.

## Verification and retained counterexample

On clean `9884f31`, the repeat-before-seal fixture called the original production
initializer twice. The serial trace printed `IRQ_READY` twice and reached
`INTERRUPT_ADMISSION_RETURNED`. The deliberately failing evidence remains at
`build/interrupt-admission-evidence/1791680141130631000/repeat-before-seal/`;
it is a reproduced invalid continuation, never passing qualification.

On clean `4f9f4f7`, all four new fixtures passed:

| Case | Admission boundary |
| --- | --- |
| before-arch | Controller setup before architecture initialization |
| repeat-before-seal | Second controller setup while the IDT is still writable |
| repeat-after-seal | Second setup after IDT protection, mapping sealing and IRQ enablement |
| enable-before-init | STI request before controller setup |

Run `python3 tools/test_interrupt_admission.py`, or select one `--case`.
Fixtures insert calls in main only; production phase checks, reporter, hardware
setup and entry code remain intact. The harness requires ordered exact proof
lines and refusal, no repeated initialization, CPU fault, continued boot, reset,
exit or stopped-VM substitute. It starts QEMU through QMP and queries live status
after the deliberate halt. Source/archive/fixture/image hashes and raw build,
serial, QEMU and QMP evidence remain at
`build/interrupt-admission-evidence/1791680251579028000/`.

Eight pure host tests cover CPU identity, boot/table admission, every wrong-order
transition, incomplete setup, repeated phases, readiness and competing boot or
controller claimants. All eight passed pinned strict-provenance Miri with seed 1;
its recorded thread schedule is not exhaustive concurrency exploration or AP
execution. Six Python tests reject missing, duplicate, embedded, reordered,
wrong-phase and continued records and check call-only fixture construction.
The five original BSP fixtures also passed on `4f9f4f7`. See
[verification](VERIFICATION.md) for complete counts and execution identities.

The four new independent Foundation lanes are included in the 80-check versioned
required policy. Every unrelated protection field is preserved. Local QEMU
11.1.2 remains portability evidence against frozen `genos-q35-tcg-v3` QEMU
11.1.1. The user/kernel #XM and initial-RIP/NMI-IRET failures remain required and
unresolved. No roadmap gate or release level advances from these scoped results.
