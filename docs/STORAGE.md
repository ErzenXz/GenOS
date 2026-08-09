# Persistent storage

GenOS 0.41 completes Stage 4 with a mounted `/USER/` namespace, PCI controller discovery, conservative host repair, and explicit read-only recovery. QEMU attaches the dedicated 8 MiB `build/genos-data.img` disk to a PCI IDE controller. The kernel discovers the controller and GenOS partition, mounts the newest committed `GFS2` snapshot into the VFS, and synchronously commits successful Ring 3 mutations.

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

If a device or flush error occurs, GenOS restores the pre-mutation VFS and returns failure to the application. A crash before step 6 leaves the destination uncommitted, so the prior slot remains authoritative. At mount, the higher valid generation wins. A damaged newer generation produces `PERSISTENT_STORAGE_RECOVERED_TORN_WRITE`; a later successful mutation overwrites and repairs the damaged slot.

## Ring 3 durability proof

On a fresh disk, `SHELL.ELF` creates `/USER/SHELL.TXT`, truncates it, writes two chunks, closes it, reopens it read-only, and verifies the exact bytes `Ring 3 shell file mutation is ready.`. The first QEMU boot requires `USER_DURABLE_WRITE_OK` and the host inspector independently verifies the file in the newest raw snapshot.

The next boot mounts that snapshot. Before rewriting anything, the shell opens `/USER/SHELL.TXT` read-only and verifies the exact prior bytes, producing `USER_DURABLE_RESTORE_OK`. It then repeats its mutation-capability proof and preserves the same contents.

## Application-visible state

The kernel publishes read-only `/STORAGE.STATUS` with `state=healthy`, `state=recovered`, `state=readonly`, or `state=error`. The Ring 3 shell reads the status through its normal capability-scoped VFS path. In read-only recovery it verifies the durable file, then proves both write-file and namespace-management capabilities are denied before mutation. When both slots are corrupt, QEMU requires `USER_STORAGE_FAILURE_VISIBLE_OK` while `/TMP/SESSION.TXT` remains readable.

## Host inspection and QEMU contract

`cargo xtask inspect-data` independently parses the MBR and both `GFS2` slots. It prints each valid generation and every file or directory. `cargo xtask repair-data` repairs only an image with exactly one valid snapshot: it copies that trusted snapshot to the alternate slot, increments the generation, recalculates the checksum, writes the image, and decodes it again to verify two valid copies. Healthy images are unchanged. If no valid snapshot exists, repair refuses to write rather than discarding or inventing metadata.

`cargo xtask test` performs six boots:

1. Create the partitioned filesystem, commit a Ring 3-created file, and inspect it from the host.
2. Restore that file, complete the full runtime smoke suite, reach `GENOS_READY`, and remain interrupt-responsive.
3. Boot again with host stdin/stdout attached to COM1, send `uname`, and require the Ring 3 response after `SERIAL_RX_OK`.
4. Boot a copied type-`0x7e` image, restore the durable file, and prove persistent mutations are denied while RAM data remains readable.
5. Inject a checksum-invalid newer generation, independently repair a copy, then boot the damaged original, recover the older generation, and prove a later mutation repairs the alternate slot.
6. Boot an image with a valid MBR but both slots corrupt, surface the storage error to Ring 3, and prove temporary RAM storage still works.

The current format remains deliberately bounded. It has no allocation bitmap, extents, large files, or incremental metadata journal; each mutation commits one full snapshot. Those are later filesystem-growth concerns, not unfinished Stage 4 acceptance items.
