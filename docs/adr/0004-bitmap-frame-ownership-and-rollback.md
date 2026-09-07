# Bitmap frame ownership and transactional page-table construction

Status: Accepted in PR #7

F3 replaces the lossy 256-entry returned-frame stack with a dense allocation bitmap indexed across sorted usable regions. The kernel reserves 256 KiB of static bitmap storage, representing up to 8 GiB of usable frames across at most 64 ranges; high physical addresses and holes do not consume bitmap positions. Overlap or metadata exhaustion halts with an explicit diagnostic before granting memory. The table layout freezes after the first successful grant, including after every grant has been returned.

Each managed frame has an allocated bit. Only a live grant may be freed, so double frees, unissued frames, gaps, and reserved pages fail without changing ownership. A free-word cursor bounds allocation scanning and moves back when earlier storage is returned. There is no secondary capacity limit on reclaimed pages. Tests cover 1,025 returned frames, random fragmented workloads, map overlap/capacity failure, and layout mutation after full reclamation. The BSP initializes static storage directly, avoiding a bitmap-sized boot-stack copy.

Supervisor table cloning now uses a shared allocation/storage contract with deterministic host failure injection. On failure it recursively releases only cloned table pages; borrowed leaf frames and the source tree remain unchanged. User mapping records each newly created parent edge and revokes/free those edges in reverse order on failure. Existing image/stack construction teardown then owns the remaining published leaves.

The explicit `memory-test-faults` image fails each successive physical allocation during production ELF process construction. Every failure must return to the initial live-frame count; the first successful construction must also fully reclaim. `cargo xtask test-memory` requires the rollback marker and a working normal shell, then restores a production image without allocation injection. Host clone tests fail every allocation in a branched four-level source tree and compare its complete table set to the baseline.

This chooses bounded static metadata over dynamic bitmap bootstrapping. Supporting more than 8 GiB of usable memory requires an explicit larger bitmap or a separately reviewed metadata allocator; it cannot silently truncate the firmware map. Per-subsystem ownership tokens, sensitive-frame scrubbing, contiguous allocation, emergency reserves, and concurrent/SMP ownership remain open. Kernel/user mapping permissions remain governed by ADR 0003.
