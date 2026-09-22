# Miri and the bounded ownership pilot

This lane complements host and VM tests. It runs selected production Rust code
under pinned Miri and explores a small ownership model exhaustively. Neither
result proves the whole kernel sound or replaces CPU/device/failure evidence.

## Reproduction

Install the separate pinned interpreter toolchain once; the ordinary compiler
and project toolchain remain Rust 1.97.0:

```sh
rustup toolchain install nightly-2026-09-21 --profile minimal --component miri,rust-src
python3 tools/test_pure_rust.py
```

The command requires Python 3.12 and clean committed source. It archives that
commit, builds only in a disposable tree and retains source/archive identity,
compiler details, commands, deadlines, logs, hashes, results and counterexamples
under `build/pure-rust-evidence/<run-id>/`. `--lane miri` and `--lane model` run
independently. A missing toolchain, unsupported interpreter operation, timeout,
nonzero exit, missing/duplicate proof or unexpected test count fails the lane.
There is no silent fallback from Miri to a native run. Timed-out process groups
are terminated and reaped so an orphan interpreter cannot continue consuming CPU.

## Miri scope

The pin is `nightly-2026-09-21` (initial local interpreter compiler:
`1.100.0-nightly`, commit `bba531001`, 2026-09-20). The harness retains the full
compiler identity on every execution. Flags are
`-Zmiri-strict-provenance -Zmiri-seed=1`; default alias checking and isolation stay
enabled. No flag disables an undefined-behavior check. The host target is retained
in `rustc -Vv`; these pure modules do not execute target CPU instructions.

| Production module | Interpreted tests |
| --- | ---: |
| Typed capability registry | 3 |
| Request generation | 2 |
| Canonical path policy and VFS use | 3 |
| Frame owner/generation grants, pinning and quotas | 9 |
| Per-open diagnostic snapshots | 5 |
| Bitmap allocator, release preparation and volatile scrubbing | 14 |
| Checked user-copy range/transfer planning | 3 |
| Bootloader ELF admission and bounded copy plan | 14 |
| **Total** | **53** |

The 20,000-operation randomized allocator stress exceeded the exploratory
240-second Miri budget. That failure remains evidence, not a pass. It is an
explicit native-only stress case in this lane and is still executed with its
original assertions. The other 14 allocator tests remain interpreted. A change
to suite scope or expected counts is reviewable configuration and must retain
its reason; do not hide new failures by reducing the scope.

UEFI firmware calls, inline assembly, interrupt entry, page-table/TLB CPU effects,
MMIO/PIO, DMA and actual devices are outside this interpreter scope. Running one
seed is not exhaustive exploration of Miri's possible executions. Miri itself
[documents both the errors it detects and its limitations](https://github.com/rust-lang/miri#readme):
a passing execution does not establish Rust soundness for all callers or inputs.

## Exhaustive bounded ownership model

`tools/ownership_model` is an independent host executable depending on the actual
production `kernel` library. It uses `Ledger<2>` and `FrameAllocator<1>` with exactly
two managed 4096-byte frames and two fresh user owners. It replays every word of
length 1 through 6 over this fixed twelve-action alphabet:

1. Allocate for owner A.
2. Allocate for owner B.
3. Fail the physical allocation callback.
4. Bind the newest retained grant at virtual address `0x4000`.
5. Attempt that binding using the other owner.
6. Bind the newest grant at `0x5000`.
7. Retire its `0x4000` binding with the matching owner.
8. Attempt that retirement with the other owner.
9. Retire its `0x5000` binding with the matching owner.
10. Release the newest retained grant.
11. Release the oldest retained grant, including after physical-slot reuse.
12. Fail release preparation for the newest retained grant.

Grant-dependent operations before the first successful allocation are no-ops.
Every sequence starts from fresh production state; there is no state merging,
random sampling or heuristic pruning. Breadth by sequence length ensures a
retained failing sequence has the shortest length within this alphabet.
The run covers **3,257,436 sequences and 19,248,492 replayed transitions**.

An independent two-cell oracle tracks owner, exact grant identity, allocation and
binding state. After each transition it compares admission results, callback
reachability, stale/foreign authority, exact retirement, owner counts, bitmap
population and ledger consistency. Failed preparation must retain both grant and
frame. Release while pinned or stale must not invoke the backing-frame callback.
A fresh allocation may not reproduce any retained old grant identity.

The oracle's negative fixture deliberately bypasses the ledger for a pinned
release. It must reject `[0, 3, 9]` (allocate, bind, release) and retain
`oracle-counterexample.json`. Replaying that same sequence through the production
adapter succeeds because release is safely denied; this is an injected invalid
adapter, not a discovered production bug. An actual exploration failure writes
`counterexample.json`, reports its action indices/names and stops with a nonzero
exit. Reproduce a retained production sequence with:

```sh
cargo +1.97.0 run --release --locked --manifest-path tools/ownership_model/Cargo.toml -- --replay 0,3,9
```

This is explicit bounded state exploration, not Kani, SMT, symbolic execution or
an unbounded proof. The model does not cover larger quotas, every possible
retained-handle selection, arbitrary owner creation, counter saturation, table
frames, real PTE publication/translation retirement, byte erasure, device pins,
interrupt schedules or SMP. Other production host/Miri/VM tests cover some of
those separately; they must not be inferred from this model's sequence count.

The harness parser has its own omitted/duplicate/forged/wrong-count negative tests
in `tools/test_pure_rust_harness.py`. Its complete-run manifest is the admission
record; an individual compiler command's successful exit is not a proof by itself.
