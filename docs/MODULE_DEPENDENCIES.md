# Kernel presentation and authority direction

The serial-first kernel still compiles a dormant framebuffer path. Storage,
scheduling, process lifecycle, input transport and network code must not depend
on its renderer or recovery-shell command loop. The old path through
`display::FixedText` made VFS paths, task records and syscall requests appear to
depend on the display implementation even though they only need bounded text.

`kernel::console_text` now owns the copyable 160-byte text and line-kind values.
`kernel::geometry` owns plain coordinates and rectangles. Their interfaces have
no framebuffer, terminal or mutable authority. Display re-exports these types
for existing presentation callers; authority modules import the neutral modules
directly. This preserves the type layout and ABI while changing the dependency
direction. It does not enlarge VFS paths or the 80-byte user console syscall.

The display manager may read `&RamVfs` and `&TaskSnapshotSet` to make a view. It
does not receive mutable VFS, scheduler, process or storage authority. The
composition root and recovery shell may coordinate a user action with the
runtime; the renderer itself cannot mutate the authoritative state. Serial
diagnostic output from an authority module is a separate observability path and
must never become the operation's success condition.

`python3 tools/check_kernel_dependencies.py` checks every non-presentation Rust
source below `kernel/src/` for a direct `display`/`shell` dependency, except the
two composition roots and the recovery shell. It also rejects imports of
runtime/storage/process/network authority and mutable authority borrows inside
`kernel/src/display/`. Negative host tests prove these imports and borrows fail
the check; `.github/workflows/ci.yml` runs it in static analysis. The check is a
conservative source rule, while Rust's borrow checker enforces the immutable
view parameters. It is not a complete caller-invariant or unsafe-code audit;
those remain F4.2/F4.4.

## CPU context and bounded scheduling

`kernel::cpu_context` owns the native twenty-word saved frame, assembly offsets,
fresh register initialization and pure return checks. `userspace` supplies the
executable and stack bounds from its owned process and separately checks the
active CR3 for captured frames. A frame value or a successful bounds check does
not grant address-space authority. The selectors, flag mask, stack endpoints and
assembly frame layout are unchanged. Host tests check all twenty offsets, fresh
registers, every 16-bit selector, each flag bit and guard/range boundaries.
The fault harness still corrupts only deliberate initial return words in this
module; its validator remains production code.

`kernel::scheduler` owns a bounded round-robin cursor and immutable scan snapshot.
Each scan visits every slot once, starting after the last selected slot. Idle or
skipped scans do not advance it; invalid selections preserve it. Process dispatch
and the separate two-wakeup socket budget use independent instances. A batch
keeps its initial scan order even while the coordinator handles completions.
Readiness, process ownership, timers, dispatch, cancellation and cleanup remain
with `ProcessManager`. Host tests cover every ready subset of the current four
slots, equal bounded service, wake batches, empty/singleton capacities and cursor
overflow boundaries. This is policy evidence, not whole-system fairness or an
IRQ latency qualification.

These are scoped F4.1 extractions. Process construction, ELF mapping, lifecycle,
syscall coordination, typed process/file handles and current-process raw pointers
still need their own decomposition and ownership review.

## Native image admission

`kernel::native_image` owns the fixed image/stack addresses and a bounded,
immutable load plan. It borrows validated source slices and has no allocator,
paging, process-owner or publication interface. Its private page fields prevent
callers from constructing an unvalidated plan. Admission validates the whole
image before `userspace::build_process` creates an address-space root. The paging
adapter copies each bounded slice to an exclusively owned zeroed frame, maps the
validated permissions, and destroys the partial inactive root on failure. Stack
construction and process publication still belong to the coordinator.

The native layout and acceptance policy are preserved, including the required
sixteen initial data bytes and entry within executable memory rather than only
file-backed bytes. Eight host tests cover complete admission, every truncation,
late invalid segments, overlap/alignment/overflow, entry/data authority, the eight
executable-page budget and exact partial/BSS slices. Parser stress calls this
production seam; allocation injection distinguishes malformed-layout rejection
from valid-image OOM with budget zero. These checks do not qualify general ELF
applications or complete F4.1/F6.2.
