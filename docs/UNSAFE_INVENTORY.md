# Unsafe source inventory

`tools/check_unsafe.py` inventories lexical Rust `unsafe` blocks, functions, impls,
traits, extern blocks, attributes, and otherwise unclassified unsafe keywords.
It also records `asm!`, `global_asm!`, `naked_asm!`, and standalone `.s`, `.S`, and
`.asm` files. It uses only Python's standard library.

```sh
python3 -m unittest discover -s tools -p 'test_unsafe_inventory.py'
python3 tools/check_unsafe.py --check
python3 tools/check_unsafe.py --diff
```

`--check` and `--diff` return 1 when the inventory differs or the baseline is
missing, and 2 for a malformed baseline, unreadable source, or scanner error.
Neither changes the baseline. `--json` prints the current inventory for other
review tools. The default source root is the repository containing the script;
`--root` and `--baseline` support isolated test fixtures.

## What the baseline preserves

The checked file is `docs/unsafe-inventory.json`. Every site includes its path,
category, line/column, end line, nearby source context, and a SHA-256 digest of
its lexical construct. Every Rust or assembly source file is also hashed in full,
including comments and sources with no unsafe sites. Consequently, removing an
unsafe site, deleting a safety comment, changing a distant caller, or preserving
the count while changing a block fails the check. The baseline also records the
scanner's scope, exclusion rules, context size, and category counts; a baseline
with omitted context or a smaller scope does not match.

After reviewing a source change and its inventory diff, explicitly regenerate:

```sh
python3 tools/check_unsafe.py --diff
python3 tools/check_unsafe.py --write-baseline
python3 tools/check_unsafe.py --check
```

Review the baseline change together with the source change. Do not regenerate it
as an automatic response to a failed CI check. Baseline regeneration is an explicit
acknowledgment of changed source context; it is **not** a safety certification.
Any source edit, including an ordinary comment or test change, can require an
update because full-file context is deliberately retained by its digest.

## Scope and limitations

The scanner skips nested comments and ordinary, byte, C, and raw strings. Character
literals, lifetimes, raw identifiers, and escaped delimiters have regression tests.
Unterminated strings/comments and unbalanced inventoried constructs fail closed.
All source `cfg` branches and Rust macro definitions are included. Rust macros are
not expanded, so macro-generated unsafe operations and assembly emitted by build
scripts cannot be inferred from this inventory. Assembly files are counted once
per file, rather than pretending that an instruction count measures safety.

Directory names `.git`, `.cache`, `target`, `build`, `dist`, `out`, `generated`,
`vendor`, `node_modules`, and `__pycache__` are excluded at every depth. These exact
rules are stored in the baseline. Generated output elsewhere is deliberately
included. Source-file symlinks fail closed. Directory symlinks are not followed.
Source files must be valid UTF-8. No dependency source outside the repository is
included. If handwritten source is introduced into an excluded directory or
through a directory symlink, update the scanner scope as a reviewed code change.

This is a lexical inventory, not a Rust compiler frontend, ownership analysis,
call graph, or semantic proof. In unusual Rust type syntax, the construct digest
may cover a smaller lexical group than an entire declaration; the full-file digest
still detects every source change. The tool does not infer caller obligations or
protected invariants, and it does not mark any site as reviewed. The F4 requirement
to document those obligations and invariants for every unsafe boundary remains
open until a human-readable safety review actually records them. Such a review
must examine the complete source and callers, not just the nearby context excerpt.
