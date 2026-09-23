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
