# GenOS development benchmark

Host: macos / aarch64. QEMU emulator version 11.1.1. rustc 1.97.1 (8bab26f4f 2026-07-14).

Three separate QEMU q35 boots, 512 MiB RAM, no NIC, fresh disposable data disk each run, headless serial terminal. The normal user disk is not used. Development kernel; optimized userspace.

| Metric | Milliseconds |
|---|---:|
| Fastest boot | 3879 |
| Median boot | 3886 |
| Slowest boot | 3893 |

Timing starts before QEMU launch and ends after its serial readiness markers and process teardown. It includes firmware, kernel acceptance probes, fresh-volume initialization, and up to 100 ms harness polling. These are end-to-end development-boot observations, not kernel-only latency or comparative speed claims.

| Artifact | Bytes on disk |
|---|---:|
| `target/x86_64-unknown-none/debug/kernel` | 5381528 |
| `target/x86_64-unknown-uefi/debug/bootloader.efi` | 148480 |
| `target/x86_64-unknown-none/userspace/genos-init` | 10320 |
| `target/x86_64-unknown-none/userspace/genos-shell` | 45360 |
| `build/genos.img` | 67108864 |

The disk image is sparse; apparent size is shown, not resident memory or allocated disk blocks.

```text
sample 0: SCHED_CONTEXT_BENCH switches=64 min_pair_cycles=7000 avg_pair_cycles=9375
sample 0: SCHED_DISPATCH_BENCH dispatches=13 max_latency_ticks=11 avg_latency_milliticks=9076
sample 1: SCHED_CONTEXT_BENCH switches=64 min_pair_cycles=7000 avg_pair_cycles=8781
sample 1: SCHED_DISPATCH_BENCH dispatches=13 max_latency_ticks=11 avg_latency_milliticks=9076
sample 2: SCHED_CONTEXT_BENCH switches=64 min_pair_cycles=6000 avg_pair_cycles=7781
sample 2: SCHED_DISPATCH_BENCH dispatches=13 max_latency_ticks=11 avg_latency_milliticks=9076
```

Reproduce with `cargo xtask bench`. Raw logs: `build/serial-benchmark-0.log` through `build/serial-benchmark-2.log`.
