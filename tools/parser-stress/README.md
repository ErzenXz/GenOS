# Production parser and socket stress harness

This standalone host crate calls `kernel`'s actual public ELF, IPv4, IPv6, UDP,
TCP, ARP, checksum, TCP-classification, and socket-handle APIs through a local
path dependency. It adds no network dependencies, production allocation, or
hardware access. The host harness itself uses standard-library allocation.

```sh
cargo run --manifest-path tools/parser-stress/Cargo.toml --release --offline
cargo run --manifest-path tools/parser-stress/Cargo.toml --release --offline -- --seed 0 --iterations 100000
cargo test --manifest-path tools/parser-stress/Cargo.toml --offline
```

The default seed is `0x47454e4f53`, with 25,000 deterministic mutations. Every run
first checks 12 retained corpus inputs and every proper prefix of those inputs
(693 truncations). The corpus includes accepted ELF/network headers, overflowing
ELF offsets, truncated ELF payloads, IPv4 fragments, truncated IPv6 options,
invalid TCP offsets, and a socket lifecycle. Mutation operators truncate, insert,
flip bits, alter byte ranges, generate random bytes, and resize to boundary
lengths up to 65,536 bytes. Arithmetic overflow checks stay enabled in release.

The parser assertions check bounded borrowed slices, coherent load segments,
transport payload lengths, accepted TCP classifier properties, and basic IPv6
constraints. Valid corpus seeds force deeper parsing than uniform random bytes
alone. The socket command target exercises creation, connection, listener setup,
queue limits, shutdown, stale and forged handles, owner isolation, incarnation
isolation, and teardown. Rejected handles must leave both owners' socket state
unchanged. Each input executes at most 64 socket commands.

Use `--target elf|ipv4|ipv6|udp|tcp|socket` to focus mutation input on one target;
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
retained by the caller before cleaning the build. Eleven inputs are boundary fixtures. `socket-zero-slot.bin` is a minimized
regression from seed 0: a correctly tagged handle with slot byte zero caused
`then_some(slot - 1)` to underflow before rejection. The decoder now evaluates
the subtraction only after its range check, and host tests exercise every slot
byte while preserving live socket state. The endpoint decoder had the same
pattern and received the same correction.

## Limits

This is reproducible mutation stress, **not coverage-guided fuzzing**, exhaustive
verification, a sanitizer campaign, or proof of memory safety. The iteration
count counts mutated inputs, not discovered execution paths. The private DNS,
partition-discovery, filesystem-snapshot, and boot-validation implementations
are not exercised here; they still need host-callable production boundaries and
dedicated fuzz targets. Socket stress covers selected public operations, not
every network-completion interleaving. F6 remains open.

The local `[workspace]` keeps this host utility separate from boot-image members.
Its lockfile contains only the harness, `kernel`, and `genos_abi`. Nested build
output is excluded by this directory's `.gitignore`.
