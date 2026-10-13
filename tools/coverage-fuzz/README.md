# Host coverage fuzzing

This lane runs the production kernel's host-callable parsers and bounded state
machines through the shared assertions in `../parser-stress/src/lib.rs`. It uses
libFuzzer's edge counters and compare tracing to select new inputs. It never
boots the VM or sends network traffic. The eleven targets cover ELF and
native ELF, IPv4 and IPv6 frame admission, UDP, TCP, sockets, UEFI boot maps,
paths, RX/TX descriptor bookkeeping, and process-local handle-table operations.

From the repository root:

```sh
python3 tools/test_coverage_fuzz.py --target elf --runs 10000 --seed 47454
python3 tools/test_coverage_fuzz.py --target handles --runs 10000 --seed 47454 --offline
```

The first command downloads LLVM's `llvmorg-21.1.8` libFuzzer source at the
commit and per-file SHA-256 hashes in `runtime-lock.json`, then compiles it with
the installed `clang++`. Subsequent `--offline` runs require the verified source
cache and use Cargo's locked, offline path dependencies. Instrumented Rust uses
the repository's pinned Rust 1.97 toolchain. This lane supports Darwin and
Linux hosts with Clang, an archiver, and Python 3.12 or newer.

Qualification requires clean committed source. Each run builds from its own
retained Git archive and starts with a fresh copied seed corpus, using the
recorded nonzero seed. The runtime cache admits exactly the locked files;
unexpected files and symlinks are rejected, and only locked C++ files compile.
Cache updates are serialized and each run copies its runtime archive before use.

Each unique `build/coverage-fuzz/evidence/RUN/` directory retains source archive,
runtime archive, executable, corpus, crash artifacts, logs and a manifest.
The manifest binds source/tree, compiler, runtime, executable and input identities
with hashes. Build/download failures and partial timeout logs remain retained.
A success requires nonzero coverage and exact completed-run statistics.
`--require-growth` additionally requires new corpus discoveries; the two CI
targets use this mode. Prior runs are never overwritten.

The `handles` target executes at most 64 twenty-byte commands against two
production `HandleTable<4>` owners and independent maps. It checks full/duplicate
and zero-handle refusal, exact type/rights, revocation and owner isolation.
Its [record format](../parser-stress/src/handles.rs) does not invent generation
guarantees for the caller-supplied numeric keys.

If libFuzzer saves a failing input, retain it as a regression fixture after a
fix and replay its exact bytes with:

```sh
GENOS_FUZZ_TARGET=elf build/coverage-fuzz/evidence/RUN/genos-coverage-fuzz build/coverage-fuzz/evidence/RUN/artifacts/CRASH_FILE
```

The fuzzer uses a 65,536-byte input cap. It checks in-process logic only; CPU,
DMA, actual device interrupts, cross-process isolation, and storage durability
require their separate VM and fault tests. Partition and snapshot parsers,
DHCP/DNS, and syscall request sequences still need dedicated host-callable
targets. The F6.2 roadmap gate remains open.
