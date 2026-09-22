//! Faults run through the production write-back cache, snapshot encoder,
//! commit state machine, rollback callback, and remount decoder.
use super::*;
use std::{vec, vec::Vec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    Write(u32),
    Flush,
}

#[derive(Clone, Copy, Debug)]
enum Effect {
    Before,
    After,
    Torn(usize),
    Lost,
}

#[derive(Clone)]
struct Disk {
    media: Vec<[u8; SECTOR_BYTES]>,
    pending: Vec<[u8; SECTOR_BYTES]>,
    operations: Vec<Operation>,
    failure: Option<(usize, Effect)>,
    write_through: bool,
    queued: Vec<(u32, [u8; SECTOR_BYTES])>,
    reverse: bool,
    rotate: usize,
    flush_cut: Option<(usize, usize)>,
    read_failure: Vec<u32>,
}

impl Disk {
    fn new(write_through: bool) -> Self {
        let mut disk = Self {
            media: vec![[0; SECTOR_BYTES]; 128],
            pending: vec![[0; SECTOR_BYTES]; 128],
            operations: Vec::new(),
            failure: None,
            write_through,
            queued: Vec::new(),
            reverse: false,
            rotate: 0,
            flush_cut: None,
            read_failure: Vec::new(),
        };
        disk.media[0][450] = PARTITION_TYPE_GENOS;
        disk.media[0][454..458].copy_from_slice(&1u32.to_le_bytes());
        disk.media[0][458..462].copy_from_slice(&81u32.to_le_bytes());
        disk.media[0][510..512].copy_from_slice(&[0x55, 0xaa]);
        disk.power_cut();
        disk
    }

    fn operation(&mut self, op: Operation) -> Option<Effect> {
        let index = self.operations.len();
        self.operations.push(op);
        self.failure
            .filter(|(at, _)| *at == index)
            .map(|(_, effect)| effect)
    }

    fn power_cut(&mut self) {
        self.pending.clone_from(&self.media);
        self.queued.clear();
        self.failure = None;
        self.flush_cut = None;
    }
}

impl BlockIo for Disk {
    fn read(&mut self, lba: u32, output: &mut [u8; SECTOR_BYTES]) -> Result<(), StorageError> {
        if self.read_failure.contains(&lba) {
            output[..128].fill(0xde);
            return Err(StorageError::Device);
        }
        *output = self.pending[lba as usize];
        Ok(())
    }

    fn write(&mut self, lba: u32, data: &[u8; SECTOR_BYTES]) -> Result<(), StorageError> {
        let effect = self.operation(Operation::Write(lba));
        if matches!(effect, Some(Effect::Before)) {
            return Err(StorageError::Device);
        }
        if matches!(effect, Some(Effect::Lost)) {
            return Ok(());
        }
        let len = match effect {
            Some(Effect::Torn(len)) => len,
            _ => SECTOR_BYTES,
        };
        self.pending[lba as usize][..len].copy_from_slice(&data[..len]);
        self.queued.push((lba, self.pending[lba as usize]));
        if self.write_through {
            self.media[lba as usize][..len].copy_from_slice(&data[..len]);
        }
        if effect.is_some() {
            Err(StorageError::Device)
        } else {
            Ok(())
        }
    }

    fn flush(&mut self) -> Result<(), StorageError> {
        let effect = self.operation(Operation::Flush);
        if matches!(effect, Some(Effect::Before)) {
            return Err(StorageError::Device);
        }
        if matches!(effect, Some(Effect::Lost)) {
            self.queued.clear();
            return Ok(());
        }
        let mut writes = core::mem::take(&mut self.queued);
        if self.reverse {
            writes.reverse();
        }
        if !writes.is_empty() {
            let rotate = self.rotate % writes.len();
            writes.rotate_left(rotate);
        }
        let cut = self
            .flush_cut
            .filter(|(at, _)| *at == self.operations.len() - 1);
        for (lba, sector) in writes.iter().take(cut.map_or(writes.len(), |(_, n)| n)) {
            self.media[*lba as usize] = *sector;
        }
        if cut.is_some() {
            return Err(StorageError::Device);
        }
        if effect.is_some() {
            Err(StorageError::Device)
        } else {
            Ok(())
        }
    }
}

fn empty_fs() -> PersistentFs {
    PersistentFs {
        partition: Some(Partition {
            start_lba: 1,
            sectors: 81,
            read_only: false,
        }),
        cache: BlockCache::new(),
        active_slot: None,
        generation: 0,
        read_only: false,
        quarantine: None,
    }
}

fn namespace() -> RamVfs {
    let mut vfs = RamVfs::new();
    vfs.init_root();
    vfs.mkdir("/USER").unwrap();
    vfs.mkdir("/TMP").unwrap();
    vfs.write(TEMP_PATH, TEMP_PAYLOAD).unwrap();
    seed_status(&mut vfs, b"state=healthy");
    vfs
}

fn contents(byte: u8) -> RamVfs {
    let mut vfs = namespace();
    vfs.write(PERSISTENT_PATH, &[byte; 512]).unwrap();
    vfs.write(KEEP_PATH, KEEP_PAYLOAD).unwrap();
    vfs
}

fn baseline(write_through: bool) -> Disk {
    let mut disk = Disk::new(write_through);
    let mut fs = empty_fs();
    let mut buffer = [0; SLOT_BYTES];
    for byte in *b"ab" {
        assert_eq!(
            fs.commit_with(&contents(byte), &mut disk, &mut buffer),
            CommitOutcome::Committed
        );
    }
    disk.operations.clear();
    disk
}

fn mounted_fs() -> PersistentFs {
    let mut fs = empty_fs();
    fs.active_slot = Some(1);
    fs.generation = 2;
    fs
}

fn remount(disk: &mut Disk) -> (PersistentFs, RamVfs) {
    let mut fs = empty_fs();
    let mut buffer = [0; SLOT_BYTES];
    let mut slots = [SlotSummary {
        generation: None,
        blank: false,
        readable: true,
    }; 2];
    for (slot, summary) in slots.iter_mut().enumerate() {
        fs.read_slot_with(slot, &mut buffer, disk).unwrap();
        summary.generation = validate_snapshot(&buffer).ok();
    }
    let selected = newest_valid_slot(&slots).expect("prior generation survives");
    fs.read_slot_with(selected, &mut buffer, disk).unwrap();
    let mut vfs = namespace();
    apply_snapshot(&mut vfs, &buffer).unwrap();
    fs.active_slot = Some(selected);
    fs.generation = slots[selected].generation.unwrap();
    (fs, vfs)
}

fn production_mount(disk: &mut Disk) -> (PersistentBootState, PersistentFs, RamVfs) {
    let mut vfs = namespace();
    let (state, fs) = mount_device_with(&mut vfs, disk, &mut [0; SLOT_BYTES]);
    (state, fs, vfs)
}

fn retain_case(name: &str, disk: &Disk, fs: &PersistentFs, vfs: &RamVfs) {
    let Some(directory) = std::env::var_os("GENOS_STORAGE_CORPUS") else {
        return;
    };
    let directory = std::path::PathBuf::from(directory);
    let bytes: Vec<u8> = disk.media.iter().flatten().copied().collect();
    let mut expected = match fs.active_slot {
        Some(slot) => std::format!("slot={slot} generation={}\n", fs.generation),
        None => "unavailable\n".into(),
    };
    for node in vfs
        .list_root()
        .filter(|node| node.path().starts_with("/USER/"))
    {
        use std::fmt::Write;
        write!(
            &mut expected,
            "{} {} ",
            if node.kind() == NodeKind::Directory {
                "dir"
            } else {
                "file"
            },
            node.path()
        )
        .unwrap();
        for byte in node.data() {
            write!(&mut expected, "{byte:02x}").unwrap();
        }
        expected.push('\n');
    }
    std::fs::write(directory.join(std::format!("{name}.img")), bytes).unwrap();
    std::fs::write(directory.join(std::format!("{name}.expected")), expected).unwrap();
}

fn assert_contents(vfs: &RamVfs, byte: u8) {
    assert_eq!(vfs.read(PERSISTENT_PATH), Ok(&[byte; 512][..]));
    assert_eq!(vfs.read(KEEP_PATH), Ok(KEEP_PAYLOAD));
    assert_eq!(vfs.read(TEMP_PATH), Ok(TEMP_PAYLOAD));
}

#[test]
fn final_flush_failure_can_mean_old_or_new_media_but_never_acknowledges_success() {
    for effect in [Effect::Before, Effect::After] {
        let mut disk = baseline(false);
        let mut fs = mounted_fs();
        let mut live = contents(b'n');
        disk.failure = Some((43, effect));
        assert_eq!(
            fs.sync_with(
                &mut live,
                |vfs| *vfs = contents(b'b'),
                &mut disk,
                &mut [0; SLOT_BYTES]
            ),
            CommitOutcome::Failed(CommitFailure::Unknown)
        );
        assert_contents(&live, b'b');
        assert_eq!(
            live.read(STATUS_PATH),
            Ok(&b"state=readonly\ncommit=unknown\nview=last-acknowledged\nrecovery=remount"[..])
        );
        disk.power_cut();
        let (recovered, files) = remount(&mut disk);
        let (generation, byte) = match effect {
            Effect::Before => (2, b'b'),
            Effect::After => (3, b'n'),
            Effect::Torn(_) | Effect::Lost => unreachable!(),
        };
        assert_eq!(recovered.generation, generation);
        assert_contents(&files, byte);
    }
}

#[test]
fn every_write_and_flush_error_quarantines_without_replaying_dirty_data() {
    for write_through in [false, true] {
        let baseline = baseline(write_through);
        let mut successful = baseline.clone();
        assert_eq!(
            mounted_fs().commit_with(&contents(b'n'), &mut successful, &mut [0; SLOT_BYTES]),
            CommitOutcome::Committed
        );
        assert_eq!(successful.operations.len(), 44);
        assert_eq!(
            successful
                .operations
                .iter()
                .filter(|op| **op == Operation::Flush)
                .count(),
            3
        );
        for at in 0..successful.operations.len() {
            for effect in [Effect::Before, Effect::After] {
                let mut disk = baseline.clone();
                disk.failure = Some((at, effect));
                let mut fs = mounted_fs();
                let mut live = contents(b'n');
                let failure = if at >= 42 {
                    CommitFailure::Unknown
                } else {
                    CommitFailure::NotPublished
                };
                assert_eq!(
                    fs.sync_with(
                        &mut live,
                        |vfs| *vfs = contents(b'b'),
                        &mut disk,
                        &mut [0; SLOT_BYTES]
                    ),
                    CommitOutcome::Failed(failure),
                    "write_through={write_through} at={at} effect={effect:?}"
                );
                assert_contents(&live, b'b');
                assert!(fs.read_only());
                assert_eq!(fs.active_slot, Some(1));
                assert_eq!(fs.generation, 2);
                assert!(fs
                    .cache
                    .entries
                    .iter()
                    .all(|entry| !entry.valid && !entry.dirty));
                let calls = disk.operations.len();
                assert_eq!(
                    fs.commit_with(&contents(b'x'), &mut disk, &mut [0; SLOT_BYTES]),
                    CommitOutcome::Rejected
                );
                assert_eq!(disk.operations.len(), calls, "quarantine performs no I/O");
                disk.power_cut();
                let (recovered, files) = remount(&mut disk);
                let new_on_media = at == 43 && matches!(effect, Effect::After)
                    || write_through && (at == 43 || at == 42 && matches!(effect, Effect::After));
                assert_eq!(recovered.generation, if new_on_media { 3 } else { 2 });
                assert_contents(&files, if new_on_media { b'n' } else { b'b' });
                let (_, actual, live) = production_mount(&mut disk);
                assert_eq!(actual.generation, recovered.generation);
                assert_contents(&live, if new_on_media { b'n' } else { b'b' });
                retain_case(
                    &std::format!("error-{write_through}-{at}-{effect:?}"),
                    &disk,
                    &actual,
                    &live,
                );
            }
        }
    }
}

#[test]
fn torn_sector_and_second_failure_during_recovery_preserve_a_valid_generation() {
    let original = baseline(true);
    for at in [0, 2, 20, 40, 42] {
        for prefix in [1, 8, 24, 128, 511] {
            let mut disk = original.clone();
            // Simulate an already damaged inactive slot, then fail the next
            // attempt to replace it: recovery itself is allowed to fail.
            disk.media[2][0] ^= 0x80;
            disk.power_cut();
            let (mut fs, mut live) = remount(&mut disk);
            live.write(PERSISTENT_PATH, &[b'n'; 512]).unwrap();
            disk.failure = Some((at, Effect::Torn(prefix)));
            assert!(matches!(
                fs.sync_with(
                    &mut live,
                    |vfs| *vfs = contents(b'b'),
                    &mut disk,
                    &mut [0; SLOT_BYTES]
                ),
                CommitOutcome::Failed(_)
            ));
            assert!(fs.read_only());
            disk.power_cut();
            let (recovered, files) = remount(&mut disk);
            assert!(matches!(recovered.generation, 2 | 3));
            assert_contents(
                &files,
                if recovered.generation == 3 {
                    b'n'
                } else {
                    b'b'
                },
            );
        }
    }
}

#[test]
fn successful_flush_is_durable_and_rejections_issue_no_device_commands() {
    let mut disk = baseline(false);
    let mut fs = mounted_fs();
    let mut live = contents(b'n');
    assert_eq!(
        fs.sync_with(
            &mut live,
            |_| panic!("successful commit cannot roll back"),
            &mut disk,
            &mut [0; SLOT_BYTES]
        ),
        CommitOutcome::Committed
    );
    disk.power_cut();
    let (mut fs, files) = remount(&mut disk);
    assert_contents(&files, b'n');
    assert_eq!(fs.generation, 3);
    let calls = disk.operations.len();
    fs.generation = u64::MAX;
    assert_eq!(
        fs.commit_with(&contents(b'x'), &mut disk, &mut [0; SLOT_BYTES]),
        CommitOutcome::Rejected
    );
    assert!(!fs.read_only(), "counter overflow is a pre-I/O rejection");
    fs.generation = 3;
    assert_eq!(
        fs.commit_with(&namespace(), &mut disk, &mut [0; SLOT_BYTES]),
        CommitOutcome::Rejected
    );
    fs.read_only = true;
    assert_eq!(
        fs.commit_with(&contents(b'x'), &mut disk, &mut [0; SLOT_BYTES]),
        CommitOutcome::Rejected
    );
    let mut unavailable = PersistentFs::unavailable();
    assert!(unavailable.read_only());
    assert_eq!(
        unavailable.commit_with(&contents(b'x'), &mut disk, &mut [0; SLOT_BYTES]),
        CommitOutcome::Rejected
    );
    assert_eq!(disk.operations.len(), calls);
}

#[test]
fn failed_mount_does_not_publish_a_partial_snapshot() {
    let disk = baseline(false);
    let mut snapshot = [0; SLOT_BYTES];
    for (index, sector) in disk.media[42..82].iter().enumerate() {
        snapshot[index * SECTOR_BYTES..(index + 1) * SECTOR_BYTES].copy_from_slice(sector);
    }
    assert_eq!(validate_snapshot(&snapshot), Ok(2));
    let used = u32::from_le_bytes(snapshot[16..20].try_into().unwrap()) as usize;
    let (_, _, _, second_entry) = decode_entry(&snapshot, RECORD_HEADER_BYTES, used).unwrap();
    snapshot[second_entry + ENTRY_HEADER_BYTES] = 0xff;
    let sum = checksum(&snapshot);
    snapshot[RECORD_CHECKSUM_OFFSET..RECORD_CHECKSUM_OFFSET + 4]
        .copy_from_slice(&sum.to_le_bytes());
    // Namespace semantics are admission requirements too: a malformed newer
    // slot must not hide an older usable slot during generation selection.
    assert_eq!(
        validate_snapshot(&snapshot),
        Err(StorageError::InvalidRecord)
    );
    let mut vfs = namespace();
    let count = vfs.count();
    assert_eq!(
        apply_snapshot(&mut vfs, &snapshot),
        Err(StorageError::InvalidRecord)
    );
    assert_eq!(vfs.count(), count);
    assert!(vfs.find(PERSISTENT_PATH).is_none());
    assert_eq!(vfs.read(TEMP_PATH), Ok(TEMP_PAYLOAD));
}

#[test]
fn full_namespace_mount_failure_keeps_existing_files_and_reserved_status() {
    let mut snapshot = [0; SLOT_BYTES];
    encode_snapshot(&contents(b'n'), 3, &mut snapshot).unwrap();
    snapshot[7] = RECORD_COMMITTED;
    let sum = checksum(&snapshot);
    snapshot[RECORD_CHECKSUM_OFFSET..RECORD_CHECKSUM_OFFSET + 4]
        .copy_from_slice(&sum.to_le_bytes());
    let mut vfs = namespace();
    while vfs.count() < kernel::vfs::MAX_NODES - 1 {
        let name = std::format!("/TMP/F{}", vfs.count());
        vfs.write(&name, b"keep").unwrap();
    }
    let count = vfs.count();
    assert_eq!(apply_snapshot(&mut vfs, &snapshot), Err(StorageError::Vfs));
    assert_eq!(vfs.count(), count);
    assert!(vfs.find(PERSISTENT_PATH).is_none());
    assert!(vfs.find(KEEP_PATH).is_none());
    assert_eq!(vfs.read(STATUS_PATH), Ok(&b"state=healthy"[..]));
    assert_eq!(vfs.read(TEMP_PATH), Ok(TEMP_PAYLOAD));
}

#[test]
fn failed_partial_read_preserves_old_cache_identity_and_bytes() {
    let mut disk = Disk::new(false);
    let mut cache = BlockCache::new();
    let mut sector = [0; SECTOR_BYTES];
    for lba in 0..CACHE_ENTRIES as u32 {
        cache.read(lba, &mut sector, &mut disk).unwrap();
    }
    disk.read_failure.push(8);
    assert_eq!(
        cache.read(8, &mut sector, &mut disk),
        Err(StorageError::Device)
    );
    cache.read(0, &mut sector, &mut disk).unwrap();
    assert_eq!(
        sector, disk.media[0],
        "failed replacement cannot corrupt a cache hit"
    );
}

#[test]
fn reordered_writes_and_every_partial_flush_cut_recover_a_complete_generation() {
    // The device may reorder different sectors within one flush interval.
    // Every successful barrier still persists all accepted writes before it.
    for (reverse, rotate) in [(false, 0), (true, 0), (false, 17), (true, 11)] {
        for (operation, writes) in [(1, 1), (41, 39), (43, 1)] {
            for prefix in 0..=writes {
                let mut disk = baseline(false);
                disk.reverse = reverse;
                disk.rotate = rotate;
                disk.flush_cut = Some((operation, prefix));
                let mut fs = mounted_fs();
                let mut vfs = contents(b'n');
                let outcome = fs.sync_with(
                    &mut vfs,
                    |vfs| *vfs = contents(b'b'),
                    &mut disk,
                    &mut [0; SLOT_BYTES],
                );
                assert_eq!(
                    outcome,
                    CommitOutcome::Failed(if operation == 43 {
                        CommitFailure::Unknown
                    } else {
                        CommitFailure::NotPublished
                    })
                );
                assert!(fs.read_only());
                assert_contents(&vfs, b'b');
                disk.power_cut();
                let (_, mounted, live) = production_mount(&mut disk);
                let new = operation == 43 && prefix == 1;
                assert_eq!(mounted.generation, if new { 3 } else { 2 });
                assert_contents(&live, if new { b'n' } else { b'b' });
                retain_case(
                    &std::format!("reorder-{reverse}-{rotate}-{operation}-{prefix}"),
                    &disk,
                    &mounted,
                    &live,
                );
            }
        }
        let mut disk = baseline(false);
        disk.reverse = reverse;
        disk.rotate = rotate;
        assert_eq!(
            mounted_fs().commit_with(&contents(b'n'), &mut disk, &mut [0; SLOT_BYTES]),
            CommitOutcome::Committed
        );
        disk.power_cut();
        let (_, mounted, live) = production_mount(&mut disk);
        assert_eq!(mounted.generation, 3);
        assert_contents(&live, b'n');
    }
}

#[test]
fn lying_device_loses_acknowledged_durability_but_never_mounts_a_mixed_snapshot() {
    // Deliberately violates BlockIo's successful-write/flush contract. This is
    // detection/containment evidence, not a durability promise for lying media.
    let mut lost_acknowledged_commit = false;
    for operation in 0..44 {
        let mut disk = baseline(false);
        disk.failure = Some((operation, Effect::Lost));
        assert_eq!(
            mounted_fs().commit_with(&contents(b'n'), &mut disk, &mut [0; SLOT_BYTES]),
            CommitOutcome::Committed
        );
        disk.power_cut();
        let (_, mounted, live) = production_mount(&mut disk);
        assert!(matches!(mounted.generation, 2 | 3));
        assert_contents(&live, if mounted.generation == 3 { b'n' } else { b'b' });
        lost_acknowledged_commit |= mounted.generation == 2;
        retain_case(&std::format!("lost-{operation}"), &disk, &mounted, &live);
    }
    assert!(
        lost_acknowledged_commit,
        "test must demonstrate the limit of dishonest successful flushes"
    );
}

#[test]
fn either_or_both_corrupt_generations_and_unreadable_media_are_never_formatted() {
    for mask in 1..=3 {
        for offset in [0, 7, 20, 64, 511, 512, SLOT_BYTES - 1] {
            let mut disk = baseline(false);
            for (slot, offset_lba) in SLOT_OFFSETS.iter().enumerate() {
                if mask & (1 << slot) != 0 {
                    let lba = 1 + *offset_lba as usize + offset / SECTOR_BYTES;
                    disk.media[lba][offset % SECTOR_BYTES] ^= 0x80;
                }
            }
            disk.power_cut();
            let before = disk.media.clone();
            let (state, fs, vfs) = production_mount(&mut disk);
            assert!(
                disk.operations.is_empty(),
                "mount never repairs or formats damaged media"
            );
            assert_eq!(disk.media, before);
            if mask == 3 {
                assert_eq!(state, PersistentBootState::Unavailable);
                assert!(fs.read_only());
                assert!(vfs.find(PERSISTENT_PATH).is_none());
                assert_eq!(vfs.read(TEMP_PATH), Ok(TEMP_PAYLOAD));
            } else {
                assert_eq!(state, PersistentBootState::Recovered);
                assert_contents(&vfs, if mask == 1 { b'b' } else { b'a' });
            }
            retain_case(&std::format!("corrupt-{mask}-{offset}"), &disk, &fs, &vfs);
        }
    }
    for blank in [false, true] {
        for failed in [vec![0], vec![2], vec![42], vec![2, 42]] {
            let mut disk = if blank {
                Disk::new(false)
            } else {
                baseline(false)
            };
            disk.read_failure = failed.clone();
            let before = disk.media.clone();
            let (state, fs, _) = production_mount(&mut disk);
            assert!(disk.operations.is_empty());
            assert_eq!(disk.media, before);
            if blank || failed.contains(&0) || failed.len() == 2 {
                assert_eq!(state, PersistentBootState::Unavailable);
                assert!(fs.read_only());
            } else {
                assert_eq!(state, PersistentBootState::Recovered);
            }
        }
    }
}

#[test]
fn every_repair_failure_can_be_followed_by_another_failure_without_losing_the_source() {
    for first in 0..44 {
        let mut disk = baseline(false);
        disk.media[2][0] ^= 0x80;
        disk.power_cut();
        let (_, mut fs, mut live) = production_mount(&mut disk);
        live.write(PERSISTENT_PATH, &[b'n'; 512]).unwrap();
        disk.failure = Some((first, Effect::Before));
        assert!(matches!(
            fs.sync_with(
                &mut live,
                |vfs| *vfs = contents(b'b'),
                &mut disk,
                &mut [0; SLOT_BYTES]
            ),
            CommitOutcome::Failed(_)
        ));
        disk.power_cut();
        let (_, fs, live) = production_mount(&mut disk);
        assert_eq!(fs.generation, 2);
        assert_contents(&live, b'b');
        for second in [1, 41, 42, 43] {
            let mut again = disk.clone();
            again.operations.clear();
            again.failure = Some((second, Effect::After));
            let (_, mut fs, mut live) = production_mount(&mut again);
            live.write(PERSISTENT_PATH, &[b'r'; 512]).unwrap();
            assert!(matches!(
                fs.sync_with(
                    &mut live,
                    |vfs| *vfs = contents(b'b'),
                    &mut again,
                    &mut [0; SLOT_BYTES]
                ),
                CommitOutcome::Failed(_)
            ));
            assert_contents(&live, b'b');
            again.power_cut();
            let (_, fs, live) = production_mount(&mut again);
            assert_eq!(fs.generation, if second == 43 { 3 } else { 2 });
            assert_contents(&live, if second == 43 { b'r' } else { b'b' });
            retain_case(
                &std::format!("repeat-repair-{first}-{second}"),
                &again,
                &fs,
                &live,
            );
        }
    }
}

#[test]
fn full_volume_and_saturated_counters_keep_the_last_complete_snapshot() {
    let mut vfs = namespace();
    while vfs.count() < kernel::vfs::MAX_NODES {
        let name = std::format!("/USER/F{:02}{}", vfs.count(), "X".repeat(55));
        assert_eq!(name.len(), 64);
        vfs.write(&name, &[b'x'; 512]).unwrap();
    }
    let mut disk = Disk::new(false);
    let mut fs = empty_fs();
    assert_eq!(
        fs.commit_with(&vfs, &mut disk, &mut [0; SLOT_BYTES]),
        CommitOutcome::Committed
    );
    let before = disk.media.clone();
    assert!(vfs.write("/USER/EXTRA", b"no").is_err());
    assert_eq!(disk.media, before);
    fs.generation = u64::MAX;
    let operations = disk.operations.len();
    assert_eq!(
        fs.commit_with(&vfs, &mut disk, &mut [0; SLOT_BYTES]),
        CommitOutcome::Rejected
    );
    assert_eq!(disk.operations.len(), operations);
    assert_eq!(disk.media, before);
    fs.cache.clock = u64::MAX;
    fs.cache.hits = u64::MAX;
    fs.cache.misses = u64::MAX;
    fs.cache.writebacks = u64::MAX;
    fs.generation = 1;
    assert_eq!(
        fs.commit_with(&vfs, &mut disk, &mut [0; SLOT_BYTES]),
        CommitOutcome::Committed
    );
    disk.power_cut();
    let (_, fs, mounted) = production_mount(&mut disk);
    assert_eq!(fs.generation, 2);
    assert_eq!(mounted.count(), vfs.count());
    for node in vfs
        .list_root()
        .filter(|node| node.path().starts_with("/USER/"))
    {
        assert_eq!(mounted.read(node.path()), Ok(node.data()));
    }
    retain_case("full", &disk, &fs, &mounted);
}

#[test]
fn ata_flush_settles_then_waits_for_real_completion_and_rejects_error_or_timeout() {
    struct Control {
        statuses: std::collections::VecDeque<u8>,
        events: Vec<&'static str>,
        submitted: bool,
        settled: bool,
    }
    impl AtaControl for Control {
        fn status(&mut self) -> u8 {
            self.events.push("status");
            assert!(
                !self.submitted || self.settled,
                "stale pre-command idle is not completion"
            );
            self.statuses
                .pop_front()
                .expect("poll budget must remain bounded")
        }
        fn error(&mut self) -> u8 {
            self.events.push("error");
            4
        }
        fn command(&mut self, command: u8) {
            assert_eq!(command, 0xe7);
            self.events.push("flush");
            self.submitted = true;
        }
        fn settle(&mut self) {
            self.events.push("settle");
            self.settled = true;
        }
    }
    for (statuses, expected) in [
        (vec![0x50, 0x80, 0x89, 0x58, 0x50], Ok(())),
        (vec![0x50, 0x80, 0x51], Err(StorageError::Device)),
        (vec![0x50, 0x70], Err(StorageError::Device)),
        (
            vec![0x50, 0x80, 0x80, 0x80, 0x80],
            Err(StorageError::Device),
        ),
        (
            vec![0x50, 0x58, 0x58, 0x58, 0x58],
            Err(StorageError::Device),
        ),
        (vec![0x51], Err(StorageError::Device)),
        (vec![0xff], Err(StorageError::Device)),
        (vec![0], Err(StorageError::Device)),
    ] {
        let mut io = Control {
            statuses: statuses.into(),
            events: Vec::new(),
            submitted: false,
            settled: false,
        };
        assert_eq!(ata_flush_with(&mut io, 4), expected);
        if io.submitted {
            assert_eq!(&io.events[..3], &["status", "flush", "settle"]);
        } else {
            assert!(!io.events.contains(&"flush"));
        }
    }
}

#[test]
fn lower_case_persistent_paths_are_durable_and_keep_filename_spelling() {
    let mut disk = baseline(false);
    let mut vfs = contents(b'b');
    vfs.mkdir("/user/Mixed").unwrap();
    vfs.write("/uSeR/mixed/Notes.txt", b"do not lose").unwrap();
    assert_eq!(
        mounted_fs().commit_with(&vfs, &mut disk, &mut [0; SLOT_BYTES]),
        CommitOutcome::Committed
    );
    disk.power_cut();
    let (_, fs, live) = production_mount(&mut disk);
    assert_eq!(live.read("/USER/Mixed/Notes.txt"), Ok(&b"do not lose"[..]));
    assert!(live
        .list_root()
        .any(|node| node.path() == "/USER/mixed/Notes.txt"));
    retain_case("case-insensitive", &disk, &fs, &live);
}

fn raw_snapshot(entries: &[(&str, NodeKind, &[u8])], generation: u64) -> [u8; SLOT_BYTES] {
    let mut snapshot = [0; SLOT_BYTES];
    snapshot[..4].copy_from_slice(b"GFS2");
    snapshot[4..6].copy_from_slice(&3u16.to_le_bytes());
    snapshot[6] = entries.len() as u8;
    snapshot[7] = RECORD_COMMITTED;
    snapshot[8..16].copy_from_slice(&generation.to_le_bytes());
    let mut cursor = 64;
    for (path, kind, data) in entries {
        snapshot[cursor] = path.len() as u8;
        snapshot[cursor + 1] = if *kind == NodeKind::Directory { 2 } else { 1 };
        snapshot[cursor + 2..cursor + 4].copy_from_slice(&(data.len() as u16).to_le_bytes());
        cursor += 4;
        snapshot[cursor..cursor + path.len()].copy_from_slice(path.as_bytes());
        cursor += path.len();
        snapshot[cursor..cursor + data.len()].copy_from_slice(data);
        cursor += data.len();
    }
    snapshot[16..20].copy_from_slice(&(cursor as u32).to_le_bytes());
    let sum = checksum(&snapshot);
    snapshot[20..24].copy_from_slice(&sum.to_le_bytes());
    snapshot
}

#[test]
fn semantically_invalid_newer_slots_do_not_shadow_a_mountable_old_generation() {
    let oversized = [b'x'; 513];
    let long_path = std::format!("/USER/{}", "X".repeat(59));
    let cases = [
        vec![("/USER/..", NodeKind::File, &b"x"[..])],
        vec![("/USER//X", NodeKind::File, &b"x"[..])],
        vec![("/USER/X X", NodeKind::File, &b"x"[..])],
        vec![("/USER/é", NodeKind::File, &b"x"[..])],
        vec![("/USER/MISSING/X", NodeKind::File, &b"x"[..])],
        vec![("/USER/BIG", NodeKind::File, &oversized[..])],
        vec![(long_path.as_str(), NodeKind::File, &b"x"[..])],
        vec![
            ("/USER/X", NodeKind::File, &b"a"[..]),
            ("/USER/x", NodeKind::File, &b"b"[..]),
        ],
        vec![
            ("/USER/D/X", NodeKind::File, &b"x"[..]),
            ("/USER/D", NodeKind::Directory, &b""[..]),
        ],
        vec![
            ("/USER/D", NodeKind::File, &b"x"[..]),
            ("/USER/D/X", NodeKind::File, &b"x"[..]),
        ],
    ];
    for (index, entries) in cases.iter().enumerate() {
        let snapshot = raw_snapshot(entries, 3);
        assert_eq!(
            validate_snapshot(&snapshot),
            Err(StorageError::InvalidRecord)
        );
        let mut disk = baseline(false);
        for (index, sector) in snapshot.chunks_exact(512).enumerate() {
            disk.media[2 + index].copy_from_slice(sector);
        }
        disk.power_cut();
        let (state, fs, live) = production_mount(&mut disk);
        assert_eq!(state, PersistentBootState::Recovered);
        assert_eq!(fs.generation, 2);
        assert_contents(&live, b'b');
        retain_case(&std::format!("semantic-{index}"), &disk, &fs, &live);
    }
    let mut disk = baseline(false);
    // Equal counters choose slot zero in both implementations, never iterator
    // order's last maximum. Different payloads make a tie-policy drift visible.
    disk.media[42][8..16].copy_from_slice(&1u64.to_le_bytes());
    let mut snapshot = [0; SLOT_BYTES];
    for (index, sector) in disk.media[42..82].iter().enumerate() {
        snapshot[index * 512..(index + 1) * 512].copy_from_slice(sector);
    }
    let sum = checksum(&snapshot);
    disk.media[42][20..24].copy_from_slice(&sum.to_le_bytes());
    disk.power_cut();
    let (_, fs, live) = production_mount(&mut disk);
    assert_eq!(fs.active_slot, Some(0));
    assert_contents(&live, b'a');
    retain_case("generation-tie", &disk, &fs, &live);
}

#[test]
fn every_failed_fresh_creation_removes_unacknowledged_seeds_and_prevents_retry() {
    for write_through in [false, true] {
        for operation in 0..44 {
            for effect in [Effect::Before, Effect::After] {
                let mut disk = Disk::new(write_through);
                disk.failure = Some((operation, effect));
                let (state, mut fs, live) = production_mount(&mut disk);
                assert_eq!(state, PersistentBootState::Unavailable);
                assert!(fs.read_only());
                assert!(live.find(PERSISTENT_PATH).is_none());
                assert!(live.find(KEEP_PATH).is_none());
                assert_eq!(live.read(TEMP_PATH), Ok(TEMP_PAYLOAD));
                let calls = disk.operations.len();
                assert_eq!(
                    fs.commit_with(&contents(b'x'), &mut disk, &mut [0; SLOT_BYTES]),
                    CommitOutcome::Rejected
                );
                assert_eq!(disk.operations.len(), calls);
                // A fresh mount may reveal a fully published seed generation,
                // but a failed creation never advertises those files in RAM.
                disk.power_cut();
                let mut snapshot = [0; SLOT_BYTES];
                for (index, sector) in disk.media[2..42].iter().enumerate() {
                    snapshot[index * 512..(index + 1) * 512].copy_from_slice(sector);
                }
                if validate_snapshot(&snapshot).is_ok() {
                    let mut recovered = namespace();
                    apply_snapshot(&mut recovered, &snapshot).unwrap();
                    assert_eq!(recovered.read(PERSISTENT_PATH), Ok(PERSISTENT_PAYLOAD));
                    assert_eq!(recovered.read(KEEP_PATH), Ok(KEEP_PAYLOAD));
                }
            }
        }
    }
}
