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
}

#[derive(Clone)]
struct Disk {
    media: Vec<[u8; SECTOR_BYTES]>,
    pending: Vec<[u8; SECTOR_BYTES]>,
    operations: Vec<Operation>,
    failure: Option<(usize, Effect)>,
    write_through: bool,
}

impl Disk {
    fn new(write_through: bool) -> Self {
        Self {
            media: vec![[0; SECTOR_BYTES]; 128],
            pending: vec![[0; SECTOR_BYTES]; 128],
            operations: Vec::new(),
            failure: None,
            write_through,
        }
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
        self.failure = None;
    }
}

impl BlockIo for Disk {
    fn read(&mut self, lba: u32, output: &mut [u8; SECTOR_BYTES]) -> Result<(), StorageError> {
        *output = self.pending[lba as usize];
        Ok(())
    }

    fn write(&mut self, lba: u32, data: &[u8; SECTOR_BYTES]) -> Result<(), StorageError> {
        let effect = self.operation(Operation::Write(lba));
        if matches!(effect, Some(Effect::Before)) {
            return Err(StorageError::Device);
        }
        let len = match effect {
            Some(Effect::Torn(len)) => len,
            _ => SECTOR_BYTES,
        };
        self.pending[lba as usize][..len].copy_from_slice(&data[..len]);
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
        self.media.clone_from(&self.pending);
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
            Effect::Torn(_) => unreachable!(),
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
    // The checksum and structural framing pass; namespace validation fails
    // only after the first file would previously have become visible.
    assert_eq!(validate_snapshot(&snapshot), Ok(2));
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
