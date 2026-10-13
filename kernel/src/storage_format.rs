//! Bounded GFS2 admission and transactional RAM mounting shared by ATA and host tests.
//! This module issues no device commands and grants no disk write authority.

use crate::vfs::{NodeKind, RamVfs};

pub const SECTOR_BYTES: usize = 512;
pub const PARTITION_TYPE_GENOS: u8 = 0x7f;
pub const PARTITION_TYPE_GENOS_READ_ONLY: u8 = 0x7e;
pub const PARTITION_ENTRY_OFFSET: usize = 446;
pub const PARTITION_ENTRY_BYTES: usize = 16;

pub const RECORD_MAGIC: [u8; 4] = *b"GFS2";
pub const RECORD_VERSION: u16 = 3;
pub const RECORD_COMMITTED: u8 = 0xa5;
pub const RECORD_HEADER_BYTES: usize = 64;
pub const RECORD_CHECKSUM_OFFSET: usize = 20;
pub const ENTRY_HEADER_BYTES: usize = 4;
pub const SLOT_SECTORS: u32 = 40;
pub const SLOT_BYTES: usize = SLOT_SECTORS as usize * SECTOR_BYTES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageError {
    Device,
    InvalidPartition,
    InvalidRecord,
    NoSpace,
    Vfs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Partition {
    pub start_lba: u32,
    pub sectors: u32,
    pub read_only: bool,
}

pub fn parse_partition(mbr: &[u8]) -> Result<Partition, StorageError> {
    if mbr.len() != SECTOR_BYTES {
        return Err(StorageError::InvalidPartition);
    }
    if mbr[510..512] != [0x55, 0xaa] {
        return Err(StorageError::InvalidPartition);
    }
    for index in 0..4 {
        let offset = PARTITION_ENTRY_OFFSET + index * PARTITION_ENTRY_BYTES;
        let partition_type = mbr[offset + 4];
        if !matches!(
            partition_type,
            PARTITION_TYPE_GENOS | PARTITION_TYPE_GENOS_READ_ONLY
        ) {
            continue;
        }
        let start_lba = u32::from_le_bytes(
            mbr[offset + 8..offset + 12]
                .try_into()
                .map_err(|_| StorageError::InvalidPartition)?,
        );
        let sectors = u32::from_le_bytes(
            mbr[offset + 12..offset + 16]
                .try_into()
                .map_err(|_| StorageError::InvalidPartition)?,
        );
        let required = 1 + SLOT_SECTORS * 2;
        if start_lba > 0 && sectors >= required && start_lba.checked_add(sectors).is_some() {
            return Ok(Partition {
                start_lba,
                sectors,
                read_only: partition_type == PARTITION_TYPE_GENOS_READ_ONLY,
            });
        }
    }
    Err(StorageError::InvalidPartition)
}

pub fn encode_snapshot(
    vfs: &RamVfs,
    generation: u64,
    output: &mut [u8; SLOT_BYTES],
) -> Result<(), StorageError> {
    output.fill(0);
    output[..4].copy_from_slice(&RECORD_MAGIC);
    output[4..6].copy_from_slice(&RECORD_VERSION.to_le_bytes());
    output[7] = 0;
    output[8..16].copy_from_slice(&generation.to_le_bytes());
    let mut cursor = RECORD_HEADER_BYTES;
    let mut entries = 0u8;
    for node in vfs.list_root() {
        if !crate::path_policy::is_user_writable_path(node.path()) {
            continue;
        }
        let path = node.path().as_bytes();
        let data = match node.kind() {
            NodeKind::File => node.data(),
            NodeKind::Directory => &[],
        };
        let end = cursor
            .checked_add(ENTRY_HEADER_BYTES + path.len() + data.len())
            .filter(|end| *end <= SLOT_BYTES)
            .ok_or(StorageError::NoSpace)?;
        if entries == u8::MAX || path.is_empty() || path.len() > u8::MAX as usize {
            return Err(StorageError::NoSpace);
        }
        output[cursor] = path.len() as u8;
        output[cursor + 1] = match node.kind() {
            NodeKind::File => 1,
            NodeKind::Directory => 2,
        };
        output[cursor + 2..cursor + 4].copy_from_slice(&(data.len() as u16).to_le_bytes());
        cursor += ENTRY_HEADER_BYTES;
        output[cursor..cursor + path.len()].copy_from_slice(path);
        // Runtime path authority and VFS lookup are case-insensitive. Preserve
        // the filename spelling while canonicalizing the format's /USER/ root,
        // so an acknowledged /user/name write cannot disappear on remount.
        output[cursor..cursor + 6].copy_from_slice(b"/USER/");
        cursor += path.len();
        output[cursor..cursor + data.len()].copy_from_slice(data);
        cursor += data.len();
        debug_assert_eq!(cursor, end);
        entries = entries.saturating_add(1);
    }
    if entries == 0 {
        return Err(StorageError::InvalidRecord);
    }
    output[6] = entries;
    output[16..20].copy_from_slice(&(cursor as u32).to_le_bytes());
    Ok(())
}

pub fn validate_snapshot(snapshot: &[u8; SLOT_BYTES]) -> Result<u64, StorageError> {
    if snapshot[..4] != RECORD_MAGIC
        || u16::from_le_bytes([snapshot[4], snapshot[5]]) != RECORD_VERSION
        || snapshot[6] == 0
        || snapshot[6] as usize > crate::vfs::MAX_NODES - 2
        || snapshot[7] != RECORD_COMMITTED
    {
        return Err(StorageError::InvalidRecord);
    }
    let used = u32::from_le_bytes(
        snapshot[16..20]
            .try_into()
            .map_err(|_| StorageError::InvalidRecord)?,
    ) as usize;
    if !(RECORD_HEADER_BYTES..=SLOT_BYTES).contains(&used) {
        return Err(StorageError::InvalidRecord);
    }
    let expected = u32::from_le_bytes(
        snapshot[RECORD_CHECKSUM_OFFSET..RECORD_CHECKSUM_OFFSET + 4]
            .try_into()
            .map_err(|_| StorageError::InvalidRecord)?,
    );
    if checksum(snapshot) != expected {
        return Err(StorageError::InvalidRecord);
    }
    let mut cursor = RECORD_HEADER_BYTES;
    let mut prior: [Option<(&str, NodeKind)>; crate::vfs::MAX_NODES] =
        [None; crate::vfs::MAX_NODES];
    for index in 0..snapshot[6] as usize {
        let (path, kind, data, next) = decode_entry(snapshot, cursor, used)?;
        let path = core::str::from_utf8(path).map_err(|_| StorageError::InvalidRecord)?;
        if !path.starts_with("/USER/")
            || !crate::path_policy::valid_absolute_path(path)
            || data.len() > crate::vfs::MAX_FILE_BYTES
            || prior[..index]
                .iter()
                .flatten()
                .any(|(other, _)| other.eq_ignore_ascii_case(path))
        {
            return Err(StorageError::InvalidRecord);
        }
        let parent = &path[..path.rfind('/').ok_or(StorageError::InvalidRecord)?];
        if parent != "/USER"
            && !prior[..index].iter().flatten().any(|(other, kind)| {
                *kind == NodeKind::Directory && other.eq_ignore_ascii_case(parent)
            })
        {
            return Err(StorageError::InvalidRecord);
        }
        prior[index] = Some((path, kind));
        cursor = next;
    }
    if cursor != used {
        return Err(StorageError::InvalidRecord);
    }
    Ok(u64::from_le_bytes(
        snapshot[8..16]
            .try_into()
            .map_err(|_| StorageError::InvalidRecord)?,
    ))
}

#[inline(never)]
pub fn apply_snapshot(vfs: &mut RamVfs, snapshot: &[u8; SLOT_BYTES]) -> Result<(), StorageError> {
    validate_snapshot(snapshot)?;
    // Decode into a bounded private namespace. A later malformed entry or
    // exhausted VFS must not expose an earlier portion of an unmounted volume.
    // This boot-only scratch value is below 32 KiB on the owned 2 MiB stack.
    let mut candidate = RamVfs::new();
    if vfs.find("/").is_some() {
        candidate.init_root();
    }
    for node in vfs.list_root() {
        match node.kind() {
            NodeKind::File => candidate.write(node.path(), node.data()),
            NodeKind::Directory => candidate.mkdir(node.path()),
        }
        .map_err(|_| StorageError::Vfs)?;
    }
    let used = u32::from_le_bytes(
        snapshot[16..20]
            .try_into()
            .map_err(|_| StorageError::InvalidRecord)?,
    ) as usize;
    let mut cursor = RECORD_HEADER_BYTES;
    for _ in 0..snapshot[6] {
        let (path, kind, data, next) = decode_entry(snapshot, cursor, used)?;
        let path = core::str::from_utf8(path).map_err(|_| StorageError::InvalidRecord)?;
        if !path.starts_with("/USER/") || candidate.find(path).is_some() {
            return Err(StorageError::InvalidRecord);
        }
        match kind {
            NodeKind::File => candidate.write(path, data).map_err(|_| StorageError::Vfs)?,
            NodeKind::Directory => candidate.mkdir(path).map_err(|_| StorageError::Vfs)?,
        }
        cursor = next;
    }
    *vfs = candidate;
    Ok(())
}

pub type DecodedEntry<'a> = (&'a [u8], NodeKind, &'a [u8], usize);

pub fn decode_entry(
    snapshot: &[u8; SLOT_BYTES],
    cursor: usize,
    used: usize,
) -> Result<DecodedEntry<'_>, StorageError> {
    if used > SLOT_BYTES
        || cursor
            .checked_add(ENTRY_HEADER_BYTES)
            .is_none_or(|end| end > used)
    {
        return Err(StorageError::InvalidRecord);
    }
    let path_len = snapshot[cursor] as usize;
    let kind = match snapshot[cursor + 1] {
        1 => NodeKind::File,
        2 => NodeKind::Directory,
        _ => return Err(StorageError::InvalidRecord),
    };
    let data_len = u16::from_le_bytes([snapshot[cursor + 2], snapshot[cursor + 3]]) as usize;
    if path_len == 0 || (kind == NodeKind::Directory && data_len != 0) {
        return Err(StorageError::InvalidRecord);
    }
    let path_start = cursor + ENTRY_HEADER_BYTES;
    let data_start = path_start
        .checked_add(path_len)
        .ok_or(StorageError::InvalidRecord)?;
    let end = data_start
        .checked_add(data_len)
        .filter(|end| *end <= used)
        .ok_or(StorageError::InvalidRecord)?;
    Ok((
        &snapshot[path_start..data_start],
        kind,
        &snapshot[data_start..end],
        end,
    ))
}

pub fn checksum(bytes: &[u8]) -> u32 {
    let mut hash = 0x811c_9dc5u32;
    for (index, byte) in bytes.iter().enumerate() {
        if (RECORD_CHECKSUM_OFFSET..RECORD_CHECKSUM_OFFSET + 4).contains(&index) {
            hash ^= 0;
        } else {
            hash ^= *byte as u32;
        }
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}
