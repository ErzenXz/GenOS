# Production parser and socket stress harness

This standalone host crate calls `kernel`'s actual public ELF, native-image admission, IPv4, IPv6, UDP,
TCP, ARP, checksum, TCP-classification, and socket-handle APIs through a local
path dependency. It also calls production GFS2, DHCP, DNS and ICMP codecs,
syscall scalar admission and endpoint lifecycle APIs. It adds no network dependencies, production allocation, or
hardware access. The host harness itself uses standard-library allocation.

```sh
cargo run --manifest-path tools/parser-stress/Cargo.toml --release --offline
cargo run --manifest-path tools/parser-stress/Cargo.toml --release --offline -- --seed 0 --iterations 100000
cargo test --manifest-path tools/parser-stress/Cargo.toml --offline
```

The default seed is `0x47454e4f53`, with 25,000 deterministic mutations. Every run
first checks 33 retained corpus inputs and every proper prefix of those inputs
(85,037 truncations). The corpus includes accepted ELF/network headers, overflowing
ELF offsets, truncated ELF payloads, IPv4 fragments, truncated IPv6 options,
invalid TCP offsets, and a socket lifecycle. Two native ELF fixtures exercise a
valid fixed-address image and rejection of W+X in its final load segment. Mutation operators truncate, insert,
flip bits, alter byte ranges, generate random bytes, and resize to boundary
lengths up to 65,536 bytes. Arithmetic overflow checks stay enabled in release.

The parser assertions check bounded borrowed slices, coherent load segments,
native page uniqueness, W^X, executable/data budgets and entry bounds,
transport payload lengths, accepted TCP classifier properties, and basic IPv6
constraints. Valid corpus seeds force deeper parsing than uniform random bytes
alone. The socket command target exercises creation, connection, listener setup,
queue limits, shutdown, stale and forged handles, owner isolation, incarnation
isolation, and teardown. Rejected handles must leave both owners' socket state
unchanged. Each input executes at most 64 socket commands.

Use `--help` to list all nineteen targets, including `partition`, `snapshot`,
`dhcp`, `dns`, `arp`, `icmp`, `syscalls` and `endpoints`, and `--target TARGET` to focus mutation input on one target;
the deterministic initial corpus still runs for all targets. `--target all` is
the default. `--iterations` accepts 0 through 10,000,000. Seeds accept decimal
or `0x` hexadecimal values, including zero.

## Replay and regression retention

A panic or invariant failure returns status 1 and saves the exact bytes plus a
replay command under `tools/parser-stress/target/failures/`, using a distinct
filename for each failure. Argument/read errors return 2; successful runs return
0. Failures report the seed, iteration, target, and input length. Replay uses raw
bytes, not a text/hex wrapper:

```sh
cargo run --manifest-path tools/parser-stress/Cargo.toml --release --offline -- --target elf --input tools/parser-stress/corpus/elf-overflow.bin --expect reject
```

`--expect accept|reject` optionally checks parser acceptance during replay and is
included in saved replay commands for corpus-expectation failures. It requires
an explicit parser target; sockets and `all` perform invariant checks instead.

After fixing a failure, minimize and copy its input into `corpus/`, add an entry
to the corpus table in `src/main.rs`, and record the bug and expected result in
this document. Files in `target/failures/` are temporary artifacts and must be
retained by the caller before cleaning the build. Thirteen inputs are boundary fixtures. `socket-zero-slot.bin` is a minimized
regression from seed 0: a correctly tagged handle with slot byte zero caused
`then_some(slot - 1)` to underflow before rejection. The decoder now evaluates
the subtraction only after its range check, and host tests exercise every slot
byte while preserving live socket state. The endpoint decoder had the same
pattern and received the same correction.

## Limits

This is reproducible mutation stress, **not coverage-guided fuzzing**, exhaustive
verification, a sanitizer campaign, or proof of memory safety. A separate
[coverage lane](../coverage-fuzz/README.md) reuses these assertions. The iteration
count counts mutated inputs, not discovered execution paths. The host-callable
GFS2 and DHCP seams are the production codecs; snapshot mutations separately
repair checksums to reach semantic entry validation. DNS rejection fixtures
retain former acceptance of error/truncated replies, invalid compression
pointers and a wrong question class. This is a bounded probe, not a general
resolver. Actual firmware exit, device I/O, user-copy and scheduler execution need
dedicated fuzz targets. Socket stress covers selected public operations, not
every network-completion interleaving. F6 remains open.

The local `[workspace]` keeps this host utility separate from boot-image members.
Its lockfile contains only local harness, kernel, ABI, shell and runtime packages. Nested build
output is excluded by this directory's `.gitignore`.
