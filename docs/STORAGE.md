# Persistent storage

GenOS 0.56 has a bounded persistent `/USER/` namespace, PCI controller discovery, conservative host repair, and explicit read-only recovery. Storage qualification and capacity growth remain open under roadmap S1/S2. QEMU attaches the dedicated 8 MiB `build/genos-data.img` disk to a PCI IDE controller. The kernel discovers the controller and GenOS partition, mounts the newest committed `GFS2` snapshot into the VFS, and synchronously commits successful Ring 3 mutations.

`/TMP/SESSION.TXT` remains session RAM and is never serialized. Initrd files at the VFS root also remain outside the persistent volume.

## Block device and partition

The host creates an MBR with signature `55 aa` and one type-`0x7f` partition beginning at LBA 64. Type `0x7e` selects explicit read-only recovery. The kernel reads sector zero through the block cache, scans all four MBR entries, validates the partition type, rejects integer overflow, and requires enough sectors for both filesystem generations before using any partition-relative address.

Boot emits `BLOCK_DEVICE_READY` and `PARTITION_DISCOVERED` only after these checks succeed. A missing or malformed partition produces `PERSISTENT_STORAGE_UNAVAILABLE`; temporary RAM storage and `/STORAGE.STATUS` remain available.

The kernel scans PCI configuration space for an IDE mass-storage function. Compatibility-mode controllers use the standard primary-channel ports; native-mode controllers derive I/O and control registers from BAR0 and BAR1. PCI I/O space is enabled before ATA commands are issued. QEMU requires `PCI_STORAGE_CONTROLLER_READY` before the mount can pass.

## Write-back cache

`PersistentFs` owns an eight-entry sector cache. Each entry records its LBA, 512 data bytes, validity, dirty state, and recency age. Reads hit an exact LBA or replace the least-recently-used entry. Dirty eviction writes the old sector before reuse. A flush writes every dirty entry and issues ATA cache flush command `0xe7`.

Partition discovery deliberately rereads the MBR through the cache and requires identical bytes, producing `BLOCK_CACHE_HIT_OK`. Snapshot commits exercise dirty eviction and explicit flushes. `BLOCK_CACHE_STATS` reports hits, misses, and writebacks for QEMU evidence.

## GFS2 snapshot format

The partition reserves two 40-sector, 20 KiB snapshot slots. Each slot contains:

- magic `GFS2` and format version `3`;
- a bounded entry count and commit byte `0xa5`;
- a 64-bit generation and 32-bit used length;
- a checksum field and reserved header bytes;
- ordered file or directory entries with kind, path length, data length, UTF-8 `/USER/` path, and bounded data;
- a 32-bit FNV-1a checksum over the complete slot with the checksum field treated as zero.

The decoder rejects unknown versions, missing commits, invalid checksums, truncated entries, invalid kinds, directory payloads, invalid UTF-8, duplicate paths, paths outside `/USER/`, and inconsistent used lengths. Directory entries remain in VFS insertion order so parents mount before descendants.

The on-disk namespace inherits current VFS bounds: at most 32 total nodes, paths of at most 64 bytes, and files of at most 512 bytes. A complete snapshot fits within one slot under those limits.

## Commit and recovery

Every successful file creation, write, truncate, directory creation, or removal below `/USER/` follows this sequence:

1. Capture the pre-mutation VFS in a kernel-owned rollback buffer.
2. Apply the mutation to the mounted VFS.
3. Select the slot opposite the active generation.
4. Write and flush an uncommitted destination header, invalidating any old generation there.
5. Write and flush every payload sector.
6. Rewrite and flush the first sector with the commit byte and checksum.
7. Return success to Ring 3 only after the commit completes.

The kernel distinguishes these outcomes:

| Outcome | RAM visible to running applications | Media after a fresh mount |
| --- | --- | --- |
| Committed | New snapshot; the operation returns success | New snapshot, assuming the device honors successful flushes |
| Rejected before I/O | Pre-mutation snapshot; the operation returns failure | Unchanged by this attempt |
| Device failure before publication | Pre-mutation snapshot; persistent volume becomes read-only | Prior active generation remains authoritative under the ordering assumptions |
| Unknown publication outcome | Pre-mutation snapshot; persistent volume becomes read-only | Either complete old or complete new generation; the failed operation is never acknowledged as durable |

A failure becomes **unknown** once the final committed header is about to enter
the cache/write path. The header or its final flush may have reached media before
a write error, timeout, or lost completion is reported. Rolling back RAM cannot
undo that publication.

Every write/flush failure quarantines the mounted volume for the remainder of the
boot. The cache discards all entries without writing them back. The active-slot
and generation counters retain their last acknowledged values. Existing writable
handles, new writable/manage opens, and namespace mutations are denied before
changing the VFS, and an attempted internal retry also performs no device I/O.
Unavailable storage likewise denies persistent mutations; it cannot silently
acknowledge `/USER/` changes as durable RAM-only writes.

Reads continue from the last acknowledged RAM snapshot, including through
already-open handles. The failed mutation returns the existing syscall failure
value; no ABI or disk-format version changes. The kernel restores RAM **before**
replacing `/STORAGE.STATUS` with the read-only diagnostic. Failed first-volume
creation removes its unacknowledged seed files from RAM. Temporary session files
remain readable.

There is no in-place reconciliation, retry, or unquarantine command. A fresh boot
with a reset/quiescent device discards the old kernel cache, reads the slots again,
and selects the highest valid generation. This can reveal a complete mutation
whose earlier caller received failure. Applications must inspect the recovered
state before retrying a non-idempotent operation. A damaged newer generation
produces `PERSISTENT_STORAGE_RECOVERED_TORN_WRITE`; a later successful mutation
overwrites and repairs the damaged slot. A failure during that replacement
quarantines the new mount again.

Commits are synchronous and bounded by ATA polling limits. There is no cancellation
point after a mutation enters the commit path; a missing caller acknowledgement
cannot be interpreted as proof that the mutation was absent from disk.

## Commit fault evidence and device assumptions

`cargo test -p kernel --lib storage_under_test` compiles the production storage,
cache, admission, mount and ATA completion code against injected I/O. Eighteen
host tests include the original final-flush ambiguity and 25 torn-sector cases,
176 failed fresh-volume creation points, failed-read cache replacement, eight
unreadable-device configurations, ATA error
and timeout phases, and the following retained recovery corpus:

| Fault family | Retained images |
| --- | ---: |
| Error before/after every one of 41 writes and three flushes, writeback/writethrough | 176 |
| Every partial flush cut in forward, reverse and two rotated sector orders | 176 |
| One silently lost write or dishonest successful flush at each operation | 44 |
| Header/payload/padding corruption in either or both generations | 21 |
| Error at every first repair operation, then another failure at each repair barrier/publication | 176 |
| Full namespace/counters, case-insensitive paths, malformed semantic entries, equal generations | 13 |
| **Total independently compared images** | **606** |

Run `python3 tools/test_storage_faults.py` to retain these raw disposable images
and each actual production mount result under `build/storage-fault-evidence/`.
The script runs `cargo xtask check-storage-corpus DIRECTORY`, whose host parser
shares no decoder/checksum implementation with the kernel. It compares selected
slot/generation and every path, kind and exact payload. An empty corpus, missing
expected result, disagreement, source change or test failure fails the campaign.
The manifest records source identity (including dirty status), commands, logs,
and hashes for every image/result; dirty development evidence is not a committed
reference qualification. Neither command opens the normal user volume.

The mount path preserves unreadable or nonblank-invalid media without issuing
writes. A partial failed device read cannot overwrite a still-valid cached
sector. Snapshot admission includes bounded canonical paths, payload sizes,
unique case-insensitive names and parent-before-child ordering, so malformed
newer metadata cannot hide a usable older generation. Tied valid generations
select slot zero deterministically in guest and host. The encoder canonicalizes
the case-insensitive persistent prefix to `/USER/`, preserving filename spelling.
Completely readable, all-zero slots in a valid writable provisioned partition
remain the existing fresh-volume creation signal; the format cannot distinguish
that state from an external tool deliberately zeroing both slots.

A successful flush must persist all preceding accepted writes; distinct sectors
may reorder within that interval. The failed operation may have reached any
modeled prefix/subset of media. Power loss discards the volatile queue. Writes
cannot damage unrelated sectors. The previous active slot therefore survives a
failed replacement, including a second failed repair. Detected I/O failure
quarantines writes, rolls RAM back and discards dirty cache entries.

The silent-loss cases deliberately violate that contract: a caller can receive
success yet remount the old state when a device lies about completion. The
modeled corrupt/torn slot is rejected, never combined with the other slot.
This demonstrates a limitation, not durability on dishonest media. FNV-1a is
not collision-free or adversarial integrity. Arbitrary cross-sector damage,
controller reset, physical power-loss behavior, device firmware and checksum
collisions remain outside the qualified reference model.

### ATA and host-cache contract

The reference path uses one selected master, 512-byte LBA28 PIO transfers and
ATA `FLUSH CACHE` (`0xe7`), with an eight-sector guest cache and QEMU
`cache=writeback`. It does not issue FUA, NCQ or DMA. Polling mode suppresses ATA
interrupts; four alternate-status PIO reads settle device selection, command
submission and transfer completion. Under the supported ATA timing contract,
four register cycles exceed the required 400 ns before command-status sampling.
The flush proof specifically rejects stale pre-command idle status, BSY with
stale error bits, unexpected DRQ, ERR, DF, absent-device status and exhausted
poll budgets. Successful read transfers also check final completion before
publishing bytes to cache.

The driver allows 1,000,000 status samples per phase, not a promised wall-clock
ATA timeout. ATA permits flushes longer than 30 seconds, so this reference-only
poll budget may reject a functioning slow physical device. An error or timeout
never becomes durable success. IDENTIFY-based device/capacity negotiation and
physical-controller timing qualification remain unsupported. The relevant
non-data protocol and flush semantics are in [ATA-6 draft, sections 8.12 and
9.4](https://www.read.seas.harvard.edu/~kohler/class/04f-aos/ref/hardware/ATA-d1410r3a.pdf).

QEMU writeback caching requires the guest to issue flushes; QEMU process exit
alone does not establish that host/device caches survived real power loss.
The host filesystem, drive firmware and flush implementation remain trusted.
See [QEMU drive cache semantics](https://www.qemu.org/docs/master/system/invocation.html).

### Cancellation and visibility audit

`RuntimeCoordinator::complete_vfs_request` validates the exact pending request
identity with `ManagedProcessManager::vfs_request_active` before any VFS mutation.
Kill/reap clears pending requests and invalidates that identity; the existing
`USER_ROLLBACK_CANCELLATION_OK` Ring 3 validation probe kills a queued writable
open and checks that it cannot remain active. Every mutation variant follows
the same pre-mutation gate. Denied/stale requests perform no snapshot I/O.

Once that gate admits a mutation, one BSP owns the coordinator and storage.
The function neither schedules another application nor accepts cancellation
between RAM mutation, synchronous commit, rollback and completion publication.
Interrupt handlers do not inspect/mutate VFS or storage. Thus applications see
one complete RAM state, even though the coordinator temporarily holds a mutated
candidate. Successful completion follows final flush; failure restores the
prior RAM state before exposing its read-only status. Close/exit/kill may run
afterward but cannot undo a committed mutation. Losing its acknowledgement
permits old or new state on remount and is not an idempotency guarantee.
There is no asynchronous storage cancellation API; future yielding I/O must
replace this ownership contract before adding cancellation points.

## Ring 3 durability proof

On a fresh disk, `SHELL.ELF` creates `/USER/SHELL.TXT`, truncates it, writes two chunks, closes it, reopens it read-only, and verifies the exact bytes `Ring 3 shell file mutation is ready.`. The first QEMU boot requires `USER_DURABLE_WRITE_OK` and the host inspector independently verifies the file in the newest raw snapshot.

The next boot mounts that snapshot. Before rewriting anything, the shell opens `/USER/SHELL.TXT` read-only and verifies the exact prior bytes, producing `USER_DURABLE_RESTORE_OK`. It then repeats its mutation-capability proof and preserves the same contents.

## Application-visible state

The kernel publishes read-only `/STORAGE.STATUS` with `state=healthy`, `state=recovered`, `state=readonly`, or `state=error`. A quarantined volume publishes additional newline-separated fields: `commit=not-published` or `commit=unknown`, `view=last-acknowledged`, and `recovery=remount`. Read it with `cat /STORAGE.STATUS`. The Ring 3 shell reads the status through its normal capability-scoped VFS path. In read-only recovery it verifies the durable file, then proves both write-file and namespace-management capabilities are denied before mutation. When both slots are corrupt, QEMU requires `USER_STORAGE_FAILURE_VISIBLE_OK` and `USER_STORAGE_UNAVAILABLE_MUTATION_DENIED_OK`: persistent mutations must fail, `/USER` must remain empty, and the exact `/TMP/SESSION.TXT` contents must remain readable.

## Host inspection and QEMU contract

`cargo xtask inspect-data` independently parses the MBR and both `GFS2` slots. It prints each valid generation and every file or directory. `cargo xtask repair-data` repairs only an image with exactly one valid snapshot: it copies that trusted snapshot to the alternate slot, increments the generation, recalculates the checksum, writes a separate temporary image in the same directory, syncs and independently reads it back, then atomically renames it over the offline original and syncs the parent directory. Six injected publication failures, each followed by another repair attempt, prove that preparation failures preserve the original bytes and post-rename failures expose a complete repaired image. A post-rename error is an uncertain host acknowledgement; inspect before retrying. This contract requires exclusive offline access and a host filesystem with atomic same-directory rename and meaningful sync. Host repair never truncates the original image. Healthy images are unchanged. If no valid snapshot exists, repair refuses to write rather than discarding or inventing metadata.

The storage sub-suite within `cargo xtask test` performs six boots:

1. Create the partitioned filesystem, commit a Ring 3-created file, and inspect it from the host.
2. Restore that file, complete the full runtime smoke suite, reach `GENOS_READY`, and remain interrupt-responsive.
3. Boot again with host stdin/stdout attached to COM1, send `uname`, and require the Ring 3 response after `SERIAL_RX_OK`.
4. Boot a copied type-`0x7e` image, restore the durable file, and prove persistent mutations are denied while RAM data remains readable.
5. Inject a checksum-invalid newer generation, independently repair a copy, then boot the damaged original, recover the older generation, and prove a later mutation repairs the alternate slot.
6. Boot an image with a valid MBR but both slots corrupt, surface the storage error to Ring 3, and prove temporary RAM storage still works.

The current format remains deliberately bounded. It has no allocation bitmap, extents, large files, or incremental metadata journal; each mutation commits one full snapshot. Useful capacity and recovery guarantees are required by S2/C5; allocation bitmaps,
extents, journaling and other commit mechanisms remain alternatives under the S2.1
design decision. The original milestone and current fault model do not establish general filesystem
reliability; physical-device qualification and fault behaviors outside the declared block contract remain open. The retained production-seam corpus supplies bounded corruption, lost/reordered-sector and independent host-checker evidence.

## Host-tool volume preservation (GenOS 0.55)

`build/genos-data.img` is the normal user's persistent volume. Builds create it only when it is absent; an unreadable or invalid existing image produces an error and is preserved for explicit recovery. `make test`, network/SDK tests, and benchmarks use separate disposable images. Storage corruption, repair, and read-only tests never target the normal volume. `make clean` removes generated artifacts while retaining `genos-data.img`. Deleting the user volume is an explicit manual reset, not a build/test side effect. Host regression tests check corrupt-image preservation and clean behavior.

ATA fallback polling now treats status bits as valid only after BSY clears and requires DRQ to match the current data/completion phase. Host tests exercise these transitions; command failures report phase, status, and the device error register, while exhausted poll budgets produce a distinct timeout diagnostic. `cargo xtask test-serial` isolates the native serial-boot gate.
