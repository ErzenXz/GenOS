# Compatibility, migration and rollback inventory

**Snapshot: GenOS 0.56 experimental console candidate, 2026-09-22.** This inventory
belongs to the source commit that contains it. The pre-campaign baseline was
`98c8b38b809f5948edf4d90cc672f22a3013bde4`; the implemented hardening series runs
through `d5a94b1` at this inventory review. Exact executed source revisions and
retained artifacts are identified separately by the release/evidence manifest. This is
not a stable-release announcement or a promise that an arbitrary older binary
can read newer state. [VERIFICATION.md](VERIFICATION.md) records executed tests;
[KNOWN_LIMITATIONS.md](KNOWN_LIMITATIONS.md) is the evolving register. The fixed
limitations snapshot below preserves what this candidate does not promise.

## Compatibility unit and ownership

Distribute and roll back the loader, kernel, initrd applications, exported SDK,
boot settings, VM profile and their hashes as one identified bundle. Persistent
user data is a separate, independently backed-up artifact; replacing a boot
image must not replace `build/genos-data.img`. Builds and `make clean` preserve
that volume. Test/benchmark/fault commands use disposable images.

The maintainers of the producing and consuming boundary jointly own changes to
its contract. No field, syscall number, handle bit pattern, disk magic or evidence
marker may acquire a different meaning without a version/migration decision.
Consumers must reject unsupported mandatory versions and features before effects.
Where that rejection is not implemented today, the support policy is **matched
bundles only**, and testing the rejection remains an explicit gap.

## Public boundary inventory

The rows inventory persisted formats, cross-binary interfaces, user-observable
interfaces and externally consumed tooling. Internal Rust module types are not
an independently supported binary interface.

| Boundary and current identity | Compatibility/migration policy | Downgrade and rollback plan | Existing evidence and remaining gap |
| --- | --- | --- | --- |
| **Firmware executable and boot volume.** `BOOTX64.EFI` on the FAT boot image; loader finds `EFI/GENOS/KERNEL.ELF` and `INITRD.GRD`, with documented `EFI/BOOT` fallback names | Regenerate the complete boot image from one commit. Keep the loader/kernel/initrd set together; filenames alone do not identify compatible contents | Stop the VM, select a retained complete boot image and its matched VM profile. Keep user data attached only under an understood read/write policy. Never replace individual files opportunistically | Actual UEFI boots and malformed-kernel fixtures exist. Exhaustive interrupted boot-image publication, physical firmware and signed activation are not implemented |
| **BootInfo and firmware map.** `GENOS_BI` magic; `BOOT_INFO_VERSION=1`, `BOOTLOADER_VERSION=1`; `repr(C)` records, bounded map and command line in `crates/abi` | A layout/semantic break bumps the handoff version and updates both producers/consumers in one bundle. Future optional extensions need explicit length/features; do not reinterpret padding | Use the old loader and old kernel together. A new loader must not synthesize an older structure by truncating fields unless a tested adapter exists | Map/version/retained-range rejection and malformed-handoff VM cases exist. Pointer accessibility and conforming firmware remain trusted; stale ExitBootServices retry exhaustion is not exhaustively injected |
| **Kernel ELF loading contract.** ELF64 LE x86-64 `ET_EXEC`, linked base 32 MiB, bounded immutable load plan, W^X and entry checks | Keep the admission policy and linker script together. Adding relocation, TLS, dynamic linking or segment types requires an explicit loader feature contract | Rebuild the old kernel with its matched linker/loader. Keep failed images and logs; never patch ELF flags merely to bypass admission | Host malformed/copy tests and six real malformed-image rejection fixtures. The QMP oracle requires explicit guest shutdown. No authenticity check or general ELF compatibility claim |
| **Initrd archive.** `GRD1`, LE file count; LE name/data lengths and raw name/data bytes | This archive has no separate negotiated feature field. A breaking layout uses a new magic; update packer/parser and applications together. It is generated immutable content, not user storage | Restore/rebuild the previous archive with the previous bundle. Do not merge archives from different SDK/runtime versions | Parser bounds and production packaging are tested. No generic initrd migration/import utility or hostile package trust boundary |
| **User syscall ABI.** `USER_ABI_VERSION=18`, `int 0x80`, register/result/error conventions in `crates/abi` and runtime | Rebuild applications against matched ABI/runtime. Keep removed calls 7/8 reserved. Preserve numbers and error values; breaking structures/semantics require a new ABI version. No promise of ABI17/19 execution on ABI18 | Downgrade kernel, shell, init and SDK together. Startup ABI mismatch exits; never translate opaque handles between kernels | Layout/constant tests, Ring 3 syscall/negative/cleanup proofs and SDK mismatch check exist. No multi-version translation layer or published support window |
| **User image layout.** `USER_IMAGE_LAYOUT_VERSION=2`; bounded ELF; eight executable pages, one data page with 16-byte kernel-owned header and four stack pages | Loader, linker script and runtime share this layout. Additional data/heap/stack growth requires explicit ABI/resource design; do not raise limits silently to accommodate tests | Use an application built with the old supported linker/runtime. Reject unsupported images; do not truncate segments or place data in guards | Actual normal/validation shell links, ELF validation and CPU guard/protection tests. General dynamic linking, TLS, heap/mmap and named persistent app loading are absent |
| **Process lifecycle and authority.** Typed opaque process/file/socket/endpoint/console handles; slot/incarnation/generation/request identities | Handles are ephemeral per process and boot, never a durable identifier. Rights cannot be inferred from numeric bits, PID, task ID or a serialized handle. Versioned structs are ABI18 data | Restart applications to obtain fresh authority. Reconnection/reopening is explicit; old requests or saved handles must fail rather than regain authority | Stale/foreign/reuse/cancellation/exit/fault/kill/reap tests. General attenuated delegation, services, process credentials and persistent capabilities are absent |
| **Console, input, IPC and status structures.** Syscalls 0–6, 9–10, 18–26 and lifecycle 29–32; bounded `repr(C)` records | Structure size, enum values, message limits, one-shot input ownership and console rights are ABI contracts. Additive fields need a versioned size/feature scheme before use | Rebuild producer and consumer together; close old channels, cancel waits and start a fresh session. No session-state restoration across reboot | Exact copy/ownership/frame tests and real console/IPC/lifecycle cases. General streams, launch arguments/environment, service discovery and terminal jobs are not present |
| **VFS paths and file operations.** Calls 11–17, 27–28, 33–34; case-insensitive absolute ASCII names, 64-byte complete paths, 128-byte I/O buffers | Preserve canonical-path rejection and capability ownership. Storage provider changes must retain explicit partial-I/O, visibility and durability semantics. Longer paths/data/streams require ABI changes where structures or limits change | Reopen paths with fresh handles. Before exporting to an older limit, report every oversized/unrepresentable entry and refuse lossy conversion by default | Namespace, rollback, rights and remount tests. No atomic replacement/rename, cwd, comprehensive metadata/permissions or large-file/stream contract |
| **GFS2 persistent disk format.** MBR GenOS partition type `0x7f`; `0x7e` read-only recovery; `GFS2` v3 dual 40-sector snapshots | Current hardening preserves the layout. Semantic admission now refuses snapshots that cannot satisfy canonical names, bounds or parent order. Lowercase runtime `/user/` prefixes are serialized as `/USER/`. Unknown format versions are refused. Growth is a separate identified format, per ADR-0008 | Preserve an offline whole-image copy before any repair/migration. For an older kernel, prefer a copied recovery image proved read-only with that exact kernel; do not assume old releases share current failure semantics. Never change version bytes to force admission | Production-seam fault corpus, independent host comparison, host repair faults and VM create/restore/recovery/read-only/corrupt phases. No in-place version migration or live reconciliation; all-zero readable slots remain ambiguous with externally erased media |
| **Host inspect/repair/image tools.** `inspect-data`, `repair-data`, disposable fault-corpus checker; raw image is the input/output contract | Inspect is read-only. Repair needs exclusive offline ownership, one trusted valid snapshot and counter space. Preparation uses same-directory temporary image, readback, file sync, rename and directory sync; failures cannot justify deleting the original | Keep the pre-repair image until independent remount/export verification. If failure follows rename, inspect before retrying: acknowledgement can be uncertain despite a complete replacement. Host rename/sync semantics are part of the support assumption | Six repair publication failures followed by another attempt, both-invalid and generation-exhaustion refusal, oracle negative tests. No lock against another writer and no physical host-power-loss qualification |
| **Diagnostic files.** `/MEMORY.STATUS` and `/STORAGE.STATUS`; read-only textual key/value records | These are diagnostic schemas without a stable version declaration. Preserve existing keys/units within this candidate; consumers ignore unknown additive keys and reject missing required keys. Breaking meaning requires a schema version or new path. Memory report bytes/stat size are immutable per open; storage status reflects current mount state | Reopen diagnostics after upgrade/downgrade; never treat an old cached sample as current allocator/storage state | Partial-read/open-handle snapshot proof, bounded report formatting and quarantine diagnostics. Text presence is not a general health or integrity certificate |
| **Network application ABI.** IPv4 configuration and bounded exchange calls 35–37, sockets/readiness 38–48; ABI18 records/flags | Keep byte-order, state/error and readiness meanings explicit. Adding IPv6 app addresses, readiness sets or larger buffers needs versioned structures/features. Never reinterpret IPv4 fields as a tagged union without negotiation | Close sockets and restart/reconnect applications. No connection/session migration across reboot or kernel replacement | UDP/DNS/TCP and bounded concurrent passive tests; close/reset/cancellation cases. General streams, full TCP profile, IPv6 application sockets and TLS remain unsupported |
| **Network wire and device contracts.** Ethernet/ARP/IPv4/DHCP/ICMP/DNS/TCP and scoped IPv6 control over modern VirtIO; ATA PIO storage, serial/input recovery devices | Wire changes must follow the relevant standard and the declared supported subset, not invent incompatible encodings. Driver feature negotiation is separate from syscall versioning. New features cannot silently change retained reference behavior | Reuse the old VM/device profile with the matched old kernel. Reset devices and reinitialize protocol state; never import live queues/DMA descriptors from another version | Scoped packet/descriptor/protocol tests and reference VM cases. Broader peer/hardware interoperability, reset/DMA containment and general Internet security are not established |
| **CPU execution contract.** x86-64, one admitted BSP; required legacy x87/MMX/SSE state, explicit optional/disabled features | CPU feature exposure is an OS policy. New enabled architectural state requires save/init/restore, exception and compatibility proofs. CPUID support alone is not an application guarantee | Select the old supported CPU profile and old complete bundle. If that profile no longer exists, refuse qualification instead of silently using a host CPU | Feature admission, context and fault fixtures exist. Arbitrary CPUs, SMP state, XSAVE extensions, TLS and performance portability are not promised |
| **Boot settings and normal/validation policy.** Fixed bounded command line; `validation-boot` Cargo feature; normal shell versus deliberate probes | There is no stable BOOT.CFG schema or runtime configuration service. Change build-time settings with loader/kernel/shell as one artifact. Kernel and shell validation features must match | Rebuild the old policy explicitly; do not infer it from optimization/debug labels. Restore normal image after test/benchmark fixtures | Binary marker exclusion and ordered normal/validation proofs. A validation image performs mutations and probes; it is not a drop-in daily-use image |
| **SDK export and host commands.** `cargo xtask new-app`, offline copied ABI/runtime/linker/toolchain; Make/xtask/Python entry points | Export includes one coherent pinned SDK; package version `0.1.0` alone does not mean ABI compatibility. Command/output changes affecting automation need documented compatibility paths or an explicit new schema | Keep previous export and toolchain lock. Rebuild from that bundle; do not update only copied ABI or runtime files. Preserve raw evidence and user data on tool failure | External offline link and guest execution/reclamation. No stable SDK release lifecycle, binary package manager, signed repository or automatic upgrades |
| **Reference/evidence interface.** `genos-q35-tcg-v3`, pinned tool/firmware identities, exact serial markers, retained commands/manifests | A reference setting change gets a new profile ID and fresh qualification. Evidence grammar changes update producer, parser and negative tests together. Markers are scoped test interfaces, not authenticated messages | Retain the old profile and artifact hashes for reproduction. Never relabel old logs as a new commit/run; failed/incomplete manifests remain failed | Fresh challenge, ordered/duplicate/stale evidence checks and unique artifacts. Remote enforcement/independent reproduction and sustained qualification must be demonstrated, not inferred from configured jobs |
| **User shell commands and serial terminal.** Documented command set, `genos>` prompt, bounded line output, ANSI clear; host Ctrl+A then x quits QEMU | Commands have their documented meanings; familiar names do not imply POSIX shell syntax. Changes to visible commands/options/errors are user-facing compatibility changes | Use the old shell with its old ABI/runtime bundle; reconnect terminal. There is no process/job/session restore or supported guest shutdown command to substitute for emulator exit | Real serial interactive acceptance. Quoting, redirection, pipelines, scripts, full escape editing and interactive job control remain open |
| **Future packages, updates, trust metadata and larger storage.** No deployed format/interface yet | Define version, mandatory features, key/time policy, downgrade protection, publication and recovery before implementation. ADR-0008 chooses future storage reuse; it does not install littlefs or migrate data | No supported automatic downgrade/update procedure exists. Keep manual immutable image selection until interruption and rollback tests establish a replacement | Design/plans only. Do not advertise planned compatibility as a current public guarantee |

Sources for numeric/layout identities are the actual
[shared ABI](../crates/abi/src/lib.rs), [initrd parser](../kernel/src/ramfs.rs),
[loader](../bootloader/src/main.rs), [storage](STORAGE.md),
[userspace contract](USERSPACE.md), [SDK](SDK.md), [boot policy](BOOT_MODES.md),
[networking](NETWORKING.md), [IPv6](IPV6.md), [CPU policy](CPU_FEATURES.md),
[terminal](TERMINAL.md), and [reference configuration](../tools/reference-vm.conf).

## Manual update and rollback procedure

1. Stop the guest and any host process writing its data image. Record the current
   bundle commit, boot/data image hashes, format/ABI versions and VM profile.
   Copy the complete data image to a separate retained location and verify the
   copy hash before changing anything. A copied live writable image is not an
   established consistent backup.
2. Build a new complete bundle separately. Preserve all previous boot/runtime
   artifacts needed for rollback. Run admission, normal boot and migration tests
   against disposable copies. No test result permits editing the normal volume
   as a fixture.
3. If the storage version is unchanged, inspect the copy with both applicable
   tools and test the target kernel's read-only mount before writable use. If it
   differs, export/import into a separate volume, retain a path/kind/length/hash
   manifest, sync and independently read back all imported content. The latter
   workflow remains S2.3 implementation work; this document is its required
   plan, not an available command.
4. Activate only a complete selected bundle and an explicitly selected volume.
   After any uncertain publication, inspect the actual selected image and data
   before repeating a non-idempotent operation. An error response is not proof
   that a persistent mutation is absent.
5. To roll back code, select the retained previous bundle. To roll back data,
   explicitly select its matching retained backup; this discards later changes
   only with the user's knowledge. Never assume code rollback reverses data.
   If newer data does not fit the older format/ABI, retain it for export and
   refuse conversion that silently drops files, rights, names or bytes.

Do not run host `repair-data` while the VM has the same volume open. Inspection
can describe a coherent snapshot only when its input is stable. The current host
repair interface has no cross-process lock; exclusive offline use is part of
its public contract.

## Fixed limitations snapshot for this candidate

This section records support limits as of the dated candidate rather than
mirroring every implementation detail in the evolving limitations register.
Newly implemented mechanisms still require their exact committed VM evidence.

- **Release/support:** Experimental, no supported daily-use/security release,
  no ABI/storage support duration, no independently reproduced required remote
  checks or completed 1000-boot/24-hour/72-hour qualification claim from this
  document. Workflow publication/enforcement must be resolved separately.
- **Hardware:** one active x86-64 BSP in the documented QEMU/OVMF configuration;
  no physical hardware matrix, SMP, IOMMU containment, general DMA pin/reset,
  NVMe/xHCI/hotplug/power-management/Wi-Fi/audio platform or arbitrary CPU claim.
- **Memory:** usable-frame representation up to 8 GiB/64 regions; 8192 live-grant
  slots, 64 frames per user owner including tables, and 256 ledger entries
  reserved from user admission in this hardening candidate. That reserve is not
  a guaranteed 256-frame pool of physically free RAM. Shared mappings, device
  pins and general runtime heap/contiguous allocation remain outside the contract.
- **Applications:** ABI18/layout2, four managed asynchronous slots including the
  shell; twenty unified handle slots per process, four file and four socket
  slots, three process handles, and bounded message/input buffers. No general
  persistent app launch, arguments/environment, heap/mmap, dynamic linker, POSIX
  compatibility layer, process identity/security domain or service supervisor.
- **Storage:** GFS2 v3 with 32 total VFS nodes, 64-byte paths and 512-byte files;
  whole-snapshot synchronous commits and no cancel point during admitted commit.
  Flush honesty and no unrelated-sector damage remain assumptions. FNV is not
  cryptographic integrity. Unknown final outcomes require remount. Blank readable
  slots cannot distinguish new provisioning from external erasure. No scalable
  file provider, atomic rename, backup/export/import command or version migration
  is delivered by ADR-0008.
- **Network:** IPv4 bounded application sockets and scoped IPv6 control-plane
  behavior. No complete long-lived TCP qualification, arbitrary-peer fairness,
  large-flight/reassembly profile, IPv6 application sockets/AAAA/IPv6-only app
  initialization, TLS or production Internet security guarantee.
- **Console:** normal Ring 3 serial shell and the documented bounded command
  set; no cwd, quoting, pipelines, redirection, scripting, mature escape parser,
  foreground cancellation/background jobs or supported guest shutdown interface.
  GUI work stays behind the console/foundation contracts.
- **Trust/updates:** no package signing, durable trust root, authenticated update
  activation, secure-time/CSPRNG/TLS policy, anti-rollback metadata or maintained
  vulnerability response/support window. Checksums and exact test markers are
  not substitutes for those facilities.
- **Evidence:** host models and VM faults establish their stated boundaries, not
  physical power loss or complete soundness. A scoped caller/unsafe review,
  coverage-guided campaigns, long-run pressure, independent reproduction and
  per-release compatibility evidence remain required where applicable.

## Release acceptance record

F7.3 is satisfied for a release only after its reviewer records that this inventory
matches that exact source, identifies every changed row and its migration plan,
retains this limitations snapshot with the release, and links the executed mixed-
version/rejection/rollback evidence appropriate to those changes. A missing
implementation is either explicitly unsupported with a safe refusal and a retained
plan, or blocks the corresponding compatibility promise. Merely adding this file,
counting its rows or having a configured workflow does not establish completion.

For this hardening candidate, require at least: matched normal/validation/SDK
bundles; malformed boot/kernel admission; stale request/handle and cleanup cases;
unchanged GFS2 v3 read/write compatibility plus read-only/corrupt/repair phases;
606-image independent recovery comparison; and retained pre-/post-change user
volume hashes demonstrating tests did not modify it. New format migration,
automated activation and reverse conversion remain unavailable, so they cannot
be reported as tested successfully.
