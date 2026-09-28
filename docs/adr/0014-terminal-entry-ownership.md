# ADR-0014: Terminal entry ownership

- **Status:** Proposed
- **Date:** 2026-09-28
- **Decision owners:** architecture/process maintainers
- **Related issue:** next-roadmap-item continuation in the local development chat
- **Related roadmap gate:** F1.4
- **Supersedes:** None; extends ADR-0002's normalized entry contract
- **Superseded by:** None

## Context and decision

A static IST top is reloaded on each entry. A fault or NMI during a Rust
handler could overwrite a live saved frame or handler locals when an entry
reused the same stack. Local IRQ masking cannot prevent those events. Return
state also lacked a complete native RFLAGS policy, and CR4.MCE was unowned.

Claim single-BSP entry ownership in assembly after saving GPRs and clearing
DF/AC, before FP helpers or Rust. Redirect all five IST slots to the existing
guarded double-fault stack while a handler owns its original frame. Every
nested entry and every #DF terminates in bounded assembly reporting. The
double-fault stack is exclusively hardware/assembly storage. First NMI/#MC
remain kernel fatal entries on their dedicated stacks.

Release ownership only after Rust has returned and any user state restoration
has completed. Restore all original IST tops before clearing the active flag;
then return or leave through the process trampoline. Validate native CS/SS,
RIP/RSP and RFLAGS before initial entry and on captured syscall/timer frames.
Enable/read back CR4.MCE only when both CPUID MCE/MCA exist. All machine checks
halt; no bank recovery contract is introduced.

## Invariants and failure

Only the admitted BSP owns the writable TSS slots and permanent guard state.
No Rust executes on the terminal stack, and no nested entry reaches FP state,
allocation, locks, process globals or ordinary dispatch. Terminal output has
fixed fields and a 65,536-poll bound per byte. UART failure halts with possibly
partial diagnostics. No teardown or recovery is attempted after kernel failure.

An NMI in a partial claim/release can use an original emergency top but still
halts before Rust. The tested handler windows begin after redirection. Actual
IRET faults after release enter the first-level kernel fatal handler; IRET
inside an active handler takes the nested path. Invalid captured user flags
terminate only that process through existing cleanup; invalid kernel-generated
context halts before admission.

## Alternatives and resource costs

Keeping static tops permits live-frame overwrite. A stack pool for recoverable
nesting would need a depth bound, frame lifetime and exhaustion/recovery policy;
current handlers offer no safe recovery contract. Per-vector terminal stacks
add storage without enabling recovery. Reusing the existing #DF stack is bounded
because it never hosts Rust. The policy adds an 88-byte configuration, five
fixed TSS writes on claim and release, and fixed ownership state. It allocates
no frames and changes no stack capacity. No latency improvement is claimed.

## Security, compatibility and rollback

User NT/privileged return state cannot reach the native return instruction
through ordinary syscall/timer capture. Deliberate kernel return corruption
still halts with real hardware evidence. Assembly uses exact Rust frame offsets,
saved GPRs and supervisor-owned permanent configuration. Public ABI, ELF,
storage and frame formats remain unchanged. Roll back a matched older bundle;
its static-stack and return-policy limitations resume and evidence must reflect
that downgrade.

Physical memory corruption, failed terminal-stack access, unrecoverable CPU
shutdown, a second #MC while MCIP is active, arbitrary event storms, SMP and
general NMI services remain deferred. This contract cannot make corrupted
machine state recoverable.

## Verification and implementation

Host tests check frame/configuration offsets and every RFLAGS bit. Harness
tests reject altered fault identity, terminal location, phase, missing/duplicate
records and false software-as-hardware reports. Nineteen immutable-source VM
fixtures cover active nesting, real #DF, failed return words, NMI re-entry,
actual #MC injection and native-state denial/cleanup. Full boot/reference and
stack-guard regressions remain required for the changed assembly boundary.

Implementation, source inventory and contracts form a reviewable local series.
Acceptance requires the exact-commit evidence in [VERIFICATION.md](../VERIFICATION.md)
and review; local tests or configured workflows alone do not make this ADR
Accepted or promote the release.

## References

- [Detailed entry/return contract and fixture matrix](../FAULT_CONTAINMENT.md)
- [Normalized entry ADR](0002-normalized-exception-entry.md)
- [CPU state ownership](../CPU_STATE.md)
- [Guarded stacks](../KERNEL_STACKS.md)
- [Intel architecture manuals](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html)
