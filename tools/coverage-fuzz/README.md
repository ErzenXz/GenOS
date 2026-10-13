# Host coverage fuzzing

This lane runs the production kernel's host-callable parsers and bounded state
machines through the shared assertions in `../parser-stress/src/lib.rs`. It uses
libFuzzer's edge counters and compare tracing to select new inputs. It never
boots the VM or sends network traffic. The ten initial targets cover ELF and
native ELF, IPv4 and IPv6 frame admission, UDP, TCP, sockets, UEFI boot maps,
paths, and RX/TX descriptor bookkeeping.

From the repository root:

```sh
python3 tools/test_coverage_fuzz.py --target elf --runs 10000
python3 tools/test_coverage_fuzz.py --target boot-map --runs 10000 --offline
```

The first command downloads LLVM's `llvmorg-21.1.8` libFuzzer source at the
commit and per-file SHA-256 hashes in `runtime-lock.json`, then compiles it with
the installed `clang++`. Subsequent `--offline` runs require the verified source
cache and use Cargo's locked, offline path dependencies. Instrumented Rust uses
the repository's pinned Rust 1.97 toolchain. This lane supports Darwin and
Linux hosts with Clang, an archiver, and Python 3.12 or newer.

The script keeps the instrumented build, per-target corpus, crash artifacts,
logs and JSON summaries under `build/coverage-fuzz/`. A success requires libFuzzer
to report nonzero coverage and a completed run. The summary records the source
revision, dirty state, compiler and runtime identities, coverage and features.
The `new_units_added` field is evidence of feedback-driven corpus discovery;
it may be zero when replaying an already saturated corpus.

If libFuzzer saves a failing input, retain it as a regression fixture after a
fix and replay its exact bytes with:

```sh
GENOS_FUZZ_TARGET=elf build/coverage-fuzz/cargo-target/$(rustc -vV | sed -n 's/^host: //p')/release/genos-coverage-fuzz build/coverage-fuzz/artifacts/elf/CRASH_FILE
```

The fuzzer uses a 65,536-byte input cap. It checks in-process logic only; CPU,
DMA, actual device interrupts, cross-process isolation, and storage durability
require their separate VM and fault tests. Partition and snapshot parsers,
DHCP/DNS, and syscall request sequences still need dedicated host-callable
targets. The F6.2 roadmap gate remains open.
