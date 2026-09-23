# Versioned reference VM

[`tools/reference-vm.conf`](../tools/reference-vm.conf) defines the candidate
`genos-q35-tcg-v3` environment. All xtask boot commands and the standalone
exception/BSP harnesses consume its machine, CPU, accelerator, memory and topology
settings. A pinned environment makes results comparable; it does not make GenOS
stable or establish that all roadmap qualification workloads have passed.

| Component | Frozen value |
| --- | --- |
| Compiler | Rust 1.97.0; complete `rustc -Vv` retained in evidence |
| QEMU reference version | 11.1.1 |
| Machine | `pc-q35-8.2` (versioned machine contract, without the rolling `q35` alias) |
| CPU | `qemu64-v1,smep=on,smap=on` |
| Execution | TCG, one emulation thread; 512 MiB; one socket/core/thread |
| UEFI firmware | SHA256 `33090cc07675baa5190d9f1e84bf5176b33bcbfa9bacac522961150cdb6dbb2a` |
| Boot volume | Snapshot-backed raw disk; explicit `ide-hd` on q35 `ide.0`, unit 0, `bootindex=1`; strict boot order |
| Data volume | Raw 8 MiB disk; separate `piix3-ide` controller and `ide-hd`, bus 0/unit 0; writeback cache |
| Console | Serial; acceptance runs have no graphical display or QEMU monitor |
| Normal debug network | No NIC |
| Normal release network | Modern VirtIO PCI; fixed MAC `52:54:00:12:34:56`; QEMU user networking |

The boot device has explicit priority so firmware does not first search the
persistent data volume for an EFI application. Boot-image writes, including
firmware `NvVars`, go to a discarded QEMU snapshot; repeated boots cannot mutate
the hashed source image or inherit a previous run's firmware state. The data
volume keeps its separate persistence behavior. Profile v2 supersedes v1's
implicit boot-device ordering and writable boot image.

The firmware digest identifies the EDK2 image supplied by the local QEMU 11.1.1
installation at `/opt/homebrew/share/qemu/edk2-x86_64-code.fd`. The path is not the
identity and is not portable. Obtain the same firmware through a trusted QEMU
package/source, retain its origin alongside release artifacts, and select it with
`GENOS_OVMF_CODE=/path/to/firmware.fd`. The repository does not redistribute that
binary or attest to its supply chain.
The demonstrated macOS host has Homebrew `qemu 11.1.1`; `rust-toolchain.toml`
requests Rust 1.97.0 and both freestanding x86_64 targets. A reproducer must
record its own package/source receipt, complete `rustc -Vv`, exact QEMU version
line and firmware digest. A matching path or package name alone is insufficient.

Profile v3 additionally enables both `ipv4=on` and `ipv6=on` explicitly. QEMU
otherwise disables IPv4 when only the IPv6 switch is supplied, rejecting the
configured IPv4 subnet/host/DNS before guest startup. Both profile parsers reject
missing, disabled or conflicting protocol switches.
This follows QEMU's [user-network option semantics](https://www.qemu.org/docs/master/system/qemu-manpage.html#network-options).

The profile spells out IPv4 subnet `10.0.2.0/24`, host `10.0.2.2`, DHCP start
`10.0.2.15`, DNS `10.0.2.3`, IPv6 prefix `fec0::/64`, host `fec0::2` and DNS
`fec0::3`. These freeze the virtual network layout. Responses from external DNS
or Internet hosts are not deterministic fixtures. The dedicated network suite
starts its own loopback service and records the dynamic forwarded port in its
actual command. No-NIC console acceptance proves independence from that service.

The CPU exposes the bounded x87/MMX/SSE/SSE2 policy; kernel initialization disables
OSXSAVE and therefore does not admit AVX/AVX-512/AMX state. The bare-metal Rust
target forbids compiler-generated SIMD in kernel code. More CPUs, ISA features,
accelerators, physical devices and firmware variants need separate qualification.
The four-vCPU BSP fixture changes only topology and still requires one admitted
kernel CPU; it is a rejection/admission test, not SMP support.

## Supported feature matrix for this candidate

"Supported" below means admitted by the named local profile and the cited
scoped evidence, not a product support promise. A variant never silently takes
the reference name: each change in machine, CPU, firmware, devices, network or
accelerator gets a new profile ID and its own evidence.

| Dimension | Admitted reference behavior | Explicitly outside this profile |
| --- | --- | --- |
| Boot and execution | x86_64 UEFI, pinned EDK2 digest, q35-8.2, TCG, 512 MiB, one BSP | Other firmware/physical machines, KVM/HVF, running application work on another CPU |
| CPU state | NX/WP, x87/MMX/SSE/SSE2, eager legacy save/restore; SMEP/SMAP both on | AVX/AVX-512/AMX, user OSXSAVE/PKE/FSGSBASE, arbitrary CPUID combinations; the feature-variant probes are tests, not alternate qualified profiles |
| Memory and faults | Bounded bitmap/grant ledger, user/kernel PTE separation, selected guard/protection faults | DMA/IOMMU isolation, SMP page retirement, arbitrary firmware aliases and complete NMI/machine-check nesting |
| Boot and persistent disks | Snapshot-backed boot IDE disk plus separately owned 8 MiB GFS2 data IDE disk | NVMe/AHCI/USB/physical power-loss guarantee, littlefs integration or automatic migration |
| Network | One modern VirtIO PCI NIC, fixed MAC, QEMU dual-stack user network; IPv4/IPv6 selected local proofs | General Internet/TLS/server support, additional NICs, offloads, physical link or arbitrary topology |
| Local console | Ring 3 serial shell, the commands and limits in [TERMINAL.md](TERMINAL.md) | GUI, POSIX shell, guest shutdown, multiuser identity, general ELF launch or full streams |

Normal debug acceptance deliberately removes the NIC; the startup-dependency
matrix also removes the data controller, leaves an empty controller, and attaches
the NIC without any host test server. These are **failure/independence probes**,
not newly supported reference layouts. They must still reach a usable local
terminal inside their stated deadline. The network suite's loopback service is
only a test fixture; its absence cannot be a precondition for local input.

## Developer and reference commands

```sh
cargo xtask reference-check
cargo xtask test-release
cargo xtask test-reference
```

`reference-check` checks the installed Rust release, exact QEMU version line and
firmware SHA256, and reports every mismatch. It does not boot an OS or qualify a
release. `test-release` permits other installed tool/firmware versions and records
`reference_environment_match=false` when they differ. An unavailable versioned
machine/CPU fails explicitly instead of silently selecting another model.

`test-reference` rejects an environment mismatch and uncommitted source, runs the
normal debug/no-NIC and release/network acceptance cases, then checks that the
source commit is still clean and unchanged. It proves only these acceptance
cases. Repeated-boot, storage recovery, network faults, CPU faults, long-running
workloads and independent reproduction remain their own gates.

A successful boot manifest records source commit and working-tree state, build
mode, run identity, profile hash, actual firmware hash/path, Rust/QEMU version
output, image hash, fresh data-image hash and exact QEMU command. It also hashes
retained serial/stderr logs. Files live under
`build/normal-evidence/<run-id>/<label>/`; the older flat paths remain copies for
existing CI collection. Early interruption leaves `status=incomplete`, and
observed boot failures record `status=failed` plus the reason. Neither is a pass.
The validation/storage/memory/SDK/serial/network launchers now retain separate
per-run artifacts under `build/validation-evidence/`; their complete record,
phase, challenge and failure contracts are described in
[VALIDATION_EVIDENCE.md](VALIDATION_EVIDENCE.md).
Python CPU fixtures retain the same environment fields in their JSON manifests,
plus their exact source patch and individual build/serial/QEMU log hashes.

## Upgrading the profile

1. Make a separate profile change with a new profile ID; explain the changed
   tool, firmware, CPU, machine, device or network contract.
2. Obtain firmware and tools from documented origins and record their complete
   versions and the firmware digest. Do not update expected hashes merely to
   silence an unexplained mismatch.
3. Run the old and new environments against the same committed source. Retain
   positive and negative CPU, storage, network, normal boot and cleanup evidence;
   explain changed behavior and every unsupported feature combination.
4. Run the roadmap qualification workloads and have another host/reviewer
   reproduce them before promoting the candidate. Record the complete profile
   and limitations with the resulting release artifacts.

The existing Linux workflow uses distribution tools and remains a developer
portability lane until its exact tool and firmware artifacts are pinned and its
reference workloads run. The versioned 8.2 machine is intentionally available on
those older hosts as well as QEMU 11.1.1; it does not disguise a version mismatch.


Storage-validation phases allow 60 seconds including firmware startup, matching
the full smoke phase. A loaded cross-architecture host was observed spending
about 20 seconds before the first loader line, exhausting the former short-phase
budget. This bounded harness deadline is not a boot-performance target; startup
performance still needs controlled workload measurements.
