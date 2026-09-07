# Explicit CPU page protections and supervisor user-copy aliases

Status: Proposed

The foundation roadmap F2 requires GenOS to establish its own CPU protection state. Firmware flags are not a kernel contract. After cloning the supervisor-only page tables, GenOS requires NX, enables and reads back EFER.NXE and CR0.WP, and enables CPUID-supported SMEP/SMAP. Optional features are reported independently; missing NX prevents untrusted execution.

User-copy routines validate the complete owned range and read/write its supervisor physical alias. This keeps the existing single-core ownership contract and avoids broad STAC windows. Every interrupt and syscall entry clears the user-controlled AC bit before Rust; the saved return frame still preserves the user's flags. Direct kernel accesses to user virtual addresses must fault with SMAP enabled.

The linker exports page-aligned text, read-only data, and mutable data boundaries. Bootstrap splits inherited 1 GiB/2 MiB mappings as needed, preserving neighbor addresses and cache/PAT attributes, then protects text RX, read-only data R/NX, and data/BSS RW/NX. The IDT's dedicated page becomes R/NX after interrupt and syscall gates are installed. New user mappings reject W+X and always mark non-executable mappings NX. Split allocation failures halt bootstrap; complete allocator rollback remains F3 work.

Alternatives considered: retaining firmware defaults cannot prove isolation; globally disabling SMAP or leaving AC set would conceal invalid kernel accesses; STAC/CLAC copy windows would require additional exception/nesting recovery state. Supervisor aliases fit the current bounded single-core mapper, but physical-page alias ownership and future per-CPU synchronization still require dedicated review. General dynamic kernel mappings, guard-paged emergency stacks, same-IST nesting, XSTATE, and SMP are not established by this change.

Acceptance uses the existing CPU-fault harness with isolated fixtures. User data/stack execution must fault at Ring 3 without harming healthy peers. Kernel IDT/text writes, user-code execution under SMEP, and direct user reads under SMAP must produce exact Ring 0 page faults and deliberate halt. These six cases run with QEMU `-cpu max`; ordinary default-CPU boots prove explicit optional-feature fallback. All eight earlier exception cases and the complete storage/network/SDK matrix remain required.

The harness refuses dirty source, records the exact commit, tool versions, patch, image hash, and command, and gives each run a unique evidence directory. A bounded UART transmit wait ensures failed diagnostic hardware cannot indefinitely prevent containment or halt.

This ADR records the concrete foundation work authorized by the user's roadmap-and-integration request. It extends ADR 0002 without reinterpreting that earlier PR's evidence. The hardware contract follows the [Intel system programming manuals](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html).
