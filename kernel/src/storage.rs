use core::ptr::addr_of_mut;

use kernel::vfs::{NodeKind, RamVfs};

use crate::{arch, serial};

const PCI_CONFIG_ADDRESS: u16 = 0x0cf8;
const PCI_CONFIG_DATA: u16 = 0x0cfc;
const ATA_COMMAND_READ: u8 = 0x20;
const ATA_COMMAND_WRITE: u8 = 0x30;
const ATA_COMMAND_FLUSH: u8 = 0xe7;
const ATA_STATUS_ERROR: u8 = 1;
const ATA_STATUS_DRQ: u8 = 1 << 3;
const ATA_STATUS_DF: u8 = 1 << 5;
const ATA_STATUS_BUSY: u8 = 1 << 7;
const ATA_POLL_LIMIT: usize = 1_000_000;

const SECTOR_BYTES: usize = 512;
const CACHE_ENTRIES: usize = 8;
const PARTITION_TYPE_GENOS: u8 = 0x7f;
const PARTITION_TYPE_GENOS_READ_ONLY: u8 = 0x7e;
const PARTITION_ENTRY_OFFSET: usize = 446;
const PARTITION_ENTRY_BYTES: usize = 16;

const RECORD_MAGIC: [u8; 4] = *b"GFS2";
const RECORD_VERSION: u16 = 3;
const RECORD_COMMITTED: u8 = 0xa5;
const RECORD_HEADER_BYTES: usize = 64;
const RECORD_CHECKSUM_OFFSET: usize = 20;
const ENTRY_HEADER_BYTES: usize = 4;
const SLOT_SECTORS: u32 = 40;
const SLOT_BYTES: usize = SLOT_SECTORS as usize * SECTOR_BYTES;
const SLOT_OFFSETS: [u32; 2] = [1, 1 + SLOT_SECTORS];

pub const PERSISTENT_PATH: &str = "/USER/PERSIST.TXT";
pub const KEEP_PATH: &str = "/USER/KEEP.TXT";
pub const STATUS_PATH: &str = "/STORAGE.STATUS";
pub const TEMP_PATH: &str = "/TMP/SESSION.TXT";
pub const PERSISTENT_PAYLOAD: &[u8] = b"GenOS persistent storage survived a reboot.";
pub const KEEP_PAYLOAD: &[u8] = b"unrelated file remains intact";
pub const TEMP_PAYLOAD: &[u8] = b"session-only RAM data";

static mut SLOT_BUFFER: [u8; SLOT_BYTES] = [0; SLOT_BYTES];
static mut ATA_IO_BASE: u16 = 0;
static mut ATA_CONTROL_BASE: u16 = 0;

#[derive(Clone, Copy)]
struct PciIdeController {
    vendor: u16,
    device: u16,
    programming_interface: u8,
    io_base: u16,
    control_base: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistentBootState {
    Created,
    Restored,
    Recovered,
    ReadOnly,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StorageError {
    Device,
    InvalidPartition,
    InvalidRecord,
    NoSpace,
    Vfs,
}

#[derive(Clone, Copy)]
struct Partition {
    start_lba: u32,
    sectors: u32,
    read_only: bool,
}

#[derive(Clone, Copy)]
struct SlotSummary {
    generation: Option<u64>,
    blank: bool,
    readable: bool,
}

#[derive(Clone, Copy)]
struct CacheEntry {
    lba: u32,
    data: [u8; SECTOR_BYTES],
    age: u64,
    valid: bool,
    dirty: bool,
}

impl CacheEntry {
    const fn empty() -> Self {
        Self {
            lba: 0,
            data: [0; SECTOR_BYTES],
            age: 0,
            valid: false,
            dirty: false,
        }
    }
}

struct BlockCache {
    entries: [CacheEntry; CACHE_ENTRIES],
    clock: u64,
    hits: u64,
    misses: u64,
    writebacks: u64,
}

impl BlockCache {
    const fn new() -> Self {
        Self {
            entries: [CacheEntry::empty(); CACHE_ENTRIES],
            clock: 0,
            hits: 0,
            misses: 0,
            writebacks: 0,
        }
    }

    fn read(&mut self, lba: u32, output: &mut [u8; SECTOR_BYTES]) -> Result<(), StorageError> {
        self.clock = self.clock.saturating_add(1);
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.valid && entry.lba == lba)
        {
            self.hits = self.hits.saturating_add(1);
            self.entries[index].age = self.clock;
            output.copy_from_slice(&self.entries[index].data);
            return Ok(());
        }
        self.misses = self.misses.saturating_add(1);
        let index = self.replacement_index();
        self.writeback(index)?;
        ata_read_sector(lba, &mut self.entries[index].data)?;
        self.entries[index].lba = lba;
        self.entries[index].age = self.clock;
        self.entries[index].valid = true;
        self.entries[index].dirty = false;
        output.copy_from_slice(&self.entries[index].data);
        Ok(())
    }

    fn write(&mut self, lba: u32, data: &[u8; SECTOR_BYTES]) -> Result<(), StorageError> {
        self.clock = self.clock.saturating_add(1);
        let index = if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.valid && entry.lba == lba)
        {
            self.hits = self.hits.saturating_add(1);
            index
        } else {
            self.misses = self.misses.saturating_add(1);
            let index = self.replacement_index();
            self.writeback(index)?;
            index
        };
        self.entries[index].lba = lba;
        self.entries[index].data.copy_from_slice(data);
        self.entries[index].age = self.clock;
        self.entries[index].valid = true;
        self.entries[index].dirty = true;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), StorageError> {
        for index in 0..self.entries.len() {
            self.writeback(index)?;
        }
        ata_flush()
    }

    fn replacement_index(&self) -> usize {
        self.entries
            .iter()
            .position(|entry| !entry.valid)
            .unwrap_or_else(|| {
                self.entries
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, entry)| entry.age)
                    .map(|(index, _)| index)
                    .unwrap_or(0)
            })
    }

    fn writeback(&mut self, index: usize) -> Result<(), StorageError> {
        if self.entries[index].valid && self.entries[index].dirty {
            ata_write_sector(self.entries[index].lba, &self.entries[index].data)?;
            self.entries[index].dirty = false;
            self.writebacks = self.writebacks.saturating_add(1);
        }
        Ok(())
    }
}

pub struct PersistentFs {
    partition: Option<Partition>,
    cache: BlockCache,
    active_slot: Option<usize>,
    generation: u64,
    read_only: bool,
}

impl PersistentFs {
    fn unavailable() -> Self {
        Self {
            partition: None,
            cache: BlockCache::new(),
            active_slot: None,
            generation: 0,
            read_only: false,
        }
    }

    pub fn sync(&mut self, vfs: &RamVfs) -> bool {
        self.commit(vfs).is_ok()
    }

    pub fn available(&self) -> bool {
        self.partition.is_some()
    }

    pub fn read_only(&self) -> bool {
        self.read_only
    }

    fn commit(&mut self, vfs: &RamVfs) -> Result<(), StorageError> {
        let partition = self.partition.ok_or(StorageError::InvalidPartition)?;
        if self.read_only {
            return Err(StorageError::Device);
        }
        let target = self.active_slot.map(|slot| slot ^ 1).unwrap_or(0);
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(StorageError::NoSpace)?;
        let buffer = unsafe { &mut *addr_of_mut!(SLOT_BUFFER) };
        encode_snapshot(vfs, generation, buffer)?;

        let base = partition
            .start_lba
            .checked_add(SLOT_OFFSETS[target])
            .ok_or(StorageError::InvalidPartition)?;
        let mut sector = [0u8; SECTOR_BYTES];

        // Invalidate the destination before any payload sector can replace the
        // previous generation. A crash anywhere before the final header write
        // leaves this slot uncommitted and the other slot authoritative.
        sector.copy_from_slice(&buffer[..SECTOR_BYTES]);
        self.cache.write(base, &sector)?;
        self.cache.flush()?;
        for index in 1..SLOT_SECTORS as usize {
            sector.copy_from_slice(&buffer[index * SECTOR_BYTES..(index + 1) * SECTOR_BYTES]);
            self.cache.write(base + index as u32, &sector)?;
        }
        self.cache.flush()?;

        buffer[7] = RECORD_COMMITTED;
        buffer[RECORD_CHECKSUM_OFFSET..RECORD_CHECKSUM_OFFSET + 4].fill(0);
        let checksum = checksum(buffer);
        buffer[RECORD_CHECKSUM_OFFSET..RECORD_CHECKSUM_OFFSET + 4]
            .copy_from_slice(&checksum.to_le_bytes());
        sector.copy_from_slice(&buffer[..SECTOR_BYTES]);
        self.cache.write(base, &sector)?;
        self.cache.flush()?;

        self.active_slot = Some(target);
        self.generation = generation;
        serial::print("PERSISTENT_COMMIT_OK generation=");
        serial::print_u64(generation);
        serial::print(" slot=");
        serial::print_u64(target as u64);
        serial::println("");
        Ok(())
    }

    fn read_slot(
        &mut self,
        slot: usize,
        output: &mut [u8; SLOT_BYTES],
    ) -> Result<(), StorageError> {
        let partition = self.partition.ok_or(StorageError::InvalidPartition)?;
        let base = partition
            .start_lba
            .checked_add(SLOT_OFFSETS[slot])
            .ok_or(StorageError::InvalidPartition)?;
        let mut sector = [0u8; SECTOR_BYTES];
        for index in 0..SLOT_SECTORS as usize {
            self.cache.read(base + index as u32, &mut sector)?;
            output[index * SECTOR_BYTES..(index + 1) * SECTOR_BYTES].copy_from_slice(&sector);
        }
        Ok(())
    }
}

pub fn init_session_ramfs(vfs: &mut RamVfs) -> bool {
    let clean = vfs.find("/TMP").is_none() && vfs.find(TEMP_PATH).is_none();
    if !clean || vfs.mkdir("/TMP").is_err() || vfs.write(TEMP_PATH, TEMP_PAYLOAD).is_err() {
        return false;
    }
    serial::println("RAMFS_TEMP_CLEAN_OK");
    serial::println("RAMFS_TEMP_READY path=/TMP/SESSION.TXT");
    true
}

pub fn mount_or_create(vfs: &mut RamVfs) -> (PersistentBootState, PersistentFs) {
    let controller = match discover_pci_ide_controller() {
        Ok(controller) => controller,
        Err(_) => {
            serial::println("PCI_STORAGE_CONTROLLER_UNAVAILABLE");
            return unavailable(vfs);
        }
    };
    unsafe {
        ATA_IO_BASE = controller.io_base;
        ATA_CONTROL_BASE = controller.control_base;
    }
    serial::print("PCI_STORAGE_CONTROLLER_READY vendor=0x");
    serial::print_hex(controller.vendor as u64);
    serial::print(" device=0x");
    serial::print_hex(controller.device as u64);
    serial::print(" prog_if=0x");
    serial::print_hex(controller.programming_interface as u64);
    serial::print(" io=0x");
    serial::print_hex(controller.io_base as u64);
    serial::print(" control=0x");
    serial::print_hex(controller.control_base as u64);
    serial::println("");
    serial::println("BLOCK_DEVICE_READY driver=ata-pio sector_bytes=512");
    let mut cache = BlockCache::new();
    let partition = match discover_partition(&mut cache) {
        Ok(partition) => partition,
        Err(_) => return unavailable(vfs),
    };
    serial::print("PARTITION_DISCOVERED scheme=mbr type=");
    if partition.read_only {
        serial::print("0x7e");
    } else {
        serial::print("0x7f");
    }
    serial::print(" start=");
    serial::print_u64(partition.start_lba as u64);
    serial::print(" sectors=");
    serial::print_u64(partition.sectors as u64);
    serial::println("");
    serial::println("BLOCK_CACHE_READY entries=8 policy=write-back");

    let mut fs = PersistentFs {
        partition: Some(partition),
        cache,
        active_slot: None,
        generation: 0,
        read_only: partition.read_only,
    };
    let mut summaries = [SlotSummary {
        generation: None,
        blank: true,
        readable: false,
    }; 2];
    for slot in 0..2 {
        let buffer = unsafe { &mut *addr_of_mut!(SLOT_BUFFER) };
        let readable = fs.read_slot(slot, buffer).is_ok();
        summaries[slot] = SlotSummary {
            generation: readable.then(|| validate_snapshot(buffer).ok()).flatten(),
            blank: readable && buffer.iter().all(|byte| *byte == 0),
            readable,
        };
    }

    if let Some(selected) = newest_valid_slot(&summaries) {
        let buffer = unsafe { &mut *addr_of_mut!(SLOT_BUFFER) };
        if fs.read_slot(selected, buffer).is_err() || apply_snapshot(vfs, buffer).is_err() {
            return unavailable(vfs);
        }
        fs.active_slot = Some(selected);
        fs.generation = summaries[selected].generation.unwrap_or(0);
        if fs.read_only {
            seed_status(vfs, b"state=readonly");
            serial::print("PERSISTENT_STORAGE_RESTORED generation=");
            serial::print_u64(fs.generation);
            serial::println("");
            serial::println("PERSISTENT_STORAGE_READ_ONLY");
            serial::println("PERSISTENT_STORAGE_READY");
            report_cache(&fs.cache);
            return (PersistentBootState::ReadOnly, fs);
        }
        let other = selected ^ 1;
        let recovered = !summaries[other].readable
            || (!summaries[other].blank && summaries[other].generation.is_none());
        if recovered {
            seed_status(vfs, b"state=recovered");
            serial::println("PERSISTENT_STORAGE_RECOVERED_TORN_WRITE");
            serial::println("CRASH_SAFE_STORAGE_READY");
            serial::println("PERSISTENT_STORAGE_READY");
            report_cache(&fs.cache);
            return (PersistentBootState::Recovered, fs);
        }
        seed_status(vfs, b"state=healthy");
        serial::print("PERSISTENT_STORAGE_RESTORED generation=");
        serial::print_u64(fs.generation);
        serial::println("");
        serial::println("PERSISTENT_STORAGE_READY");
        report_cache(&fs.cache);
        return (PersistentBootState::Restored, fs);
    }

    if !fs.read_only && summaries.iter().all(|slot| slot.readable && slot.blank) {
        if vfs.write(PERSISTENT_PATH, PERSISTENT_PAYLOAD).is_ok()
            && vfs.write(KEEP_PATH, KEEP_PAYLOAD).is_ok()
            && fs.commit(vfs).is_ok()
        {
            seed_status(vfs, b"state=healthy");
            serial::println("PERSISTENT_STORAGE_CREATED files=2 generation=1");
            serial::println("PERSISTENT_STORAGE_READY");
            report_cache(&fs.cache);
            return (PersistentBootState::Created, fs);
        }
    }

    unavailable(vfs)
}

fn unavailable(vfs: &mut RamVfs) -> (PersistentBootState, PersistentFs) {
    seed_status(vfs, b"state=error");
    serial::println("PERSISTENT_STORAGE_UNAVAILABLE");
    (
        PersistentBootState::Unavailable,
        PersistentFs::unavailable(),
    )
}

fn report_cache(cache: &BlockCache) {
    serial::print("BLOCK_CACHE_STATS hits=");
    serial::print_u64(cache.hits);
    serial::print(" misses=");
    serial::print_u64(cache.misses);
    serial::print(" writebacks=");
    serial::print_u64(cache.writebacks);
    serial::println("");
    if cache.hits > 0 {
        serial::println("BLOCK_CACHE_HIT_OK");
    }
}

fn seed_status(vfs: &mut RamVfs, status: &[u8]) {
    if vfs.write(STATUS_PATH, status).is_err() {
        serial::println("STORAGE_STATUS_VFS_FAILED");
    }
}

fn discover_pci_ide_controller() -> Result<PciIdeController, StorageError> {
    for bus in 0u16..=255 {
        for device in 0u8..32 {
            for function in 0u8..8 {
                let id = pci_read(bus as u8, device, function, 0x00);
                let vendor = id as u16;
                if vendor == u16::MAX {
                    continue;
                }
                let class = pci_read(bus as u8, device, function, 0x08);
                if (class >> 24) as u8 != 0x01 || (class >> 16) as u8 != 0x01 {
                    continue;
                }
                let programming_interface = (class >> 8) as u8;
                let (io_base, control_base) = if programming_interface & 0x01 == 0 {
                    (0x1f0, 0x3f6)
                } else {
                    let command = pci_read(bus as u8, device, function, 0x04);
                    let bar0 = pci_read(bus as u8, device, function, 0x10);
                    let bar1 = pci_read(bus as u8, device, function, 0x14);
                    let io = (bar0 & 0xfffc) as u16;
                    let control = (bar1 & 0xfffc) as u16;
                    if io == 0 || control == 0 {
                        return Err(StorageError::Device);
                    }
                    pci_write(bus as u8, device, function, 0x04, command | 1);
                    (io, control)
                };
                let command = pci_read(bus as u8, device, function, 0x04);
                pci_write(bus as u8, device, function, 0x04, command | 1);
                return Ok(PciIdeController {
                    vendor,
                    device: (id >> 16) as u16,
                    programming_interface,
                    io_base,
                    control_base,
                });
            }
        }
    }
    Err(StorageError::Device)
}

fn pci_read(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    let address = 0x8000_0000u32
        | (bus as u32) << 16
        | (device as u32) << 11
        | (function as u32) << 8
        | u32::from(offset & 0xfc);
    unsafe {
        arch::outl(PCI_CONFIG_ADDRESS, address);
        arch::inl(PCI_CONFIG_DATA)
    }
}

fn pci_write(bus: u8, device: u8, function: u8, offset: u8, value: u32) {
    let address = 0x8000_0000u32
        | (bus as u32) << 16
        | (device as u32) << 11
        | (function as u32) << 8
        | u32::from(offset & 0xfc);
    unsafe {
        arch::outl(PCI_CONFIG_ADDRESS, address);
        arch::outl(PCI_CONFIG_DATA, value);
    }
}

fn ata_port(offset: u16) -> u16 {
    unsafe { ATA_IO_BASE + offset }
}

fn discover_partition(cache: &mut BlockCache) -> Result<Partition, StorageError> {
    let mut mbr = [0u8; SECTOR_BYTES];
    cache.read(0, &mut mbr)?;
    let mut cached_mbr = [0u8; SECTOR_BYTES];
    cache.read(0, &mut cached_mbr)?;
    if cached_mbr != mbr {
        return Err(StorageError::Device);
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

fn newest_valid_slot(slots: &[SlotSummary; 2]) -> Option<usize> {
    match (slots[0].generation, slots[1].generation) {
        (Some(left), Some(right)) => Some(usize::from(right > left)),
        (Some(_), None) => Some(0),
        (None, Some(_)) => Some(1),
        (None, None) => None,
    }
}

fn encode_snapshot(
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
        if !node.path().starts_with("/USER/") {
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

fn validate_snapshot(snapshot: &[u8; SLOT_BYTES]) -> Result<u64, StorageError> {
    if snapshot[..4] != RECORD_MAGIC
        || u16::from_le_bytes([snapshot[4], snapshot[5]]) != RECORD_VERSION
        || snapshot[6] == 0
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
    for _ in 0..snapshot[6] {
        let (_, _, _, next) = decode_entry(snapshot, cursor, used)?;
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

fn apply_snapshot(vfs: &mut RamVfs, snapshot: &[u8; SLOT_BYTES]) -> Result<(), StorageError> {
    validate_snapshot(snapshot)?;
    let used = u32::from_le_bytes(
        snapshot[16..20]
            .try_into()
            .map_err(|_| StorageError::InvalidRecord)?,
    ) as usize;
    let mut cursor = RECORD_HEADER_BYTES;
    for _ in 0..snapshot[6] {
        let (path, kind, data, next) = decode_entry(snapshot, cursor, used)?;
        let path = core::str::from_utf8(path).map_err(|_| StorageError::InvalidRecord)?;
        if !path.starts_with("/USER/") || vfs.find(path).is_some() {
            return Err(StorageError::InvalidRecord);
        }
        match kind {
            NodeKind::File => vfs.write(path, data).map_err(|_| StorageError::Vfs)?,
            NodeKind::Directory => vfs.mkdir(path).map_err(|_| StorageError::Vfs)?,
        }
        cursor = next;
    }
    Ok(())
}

fn decode_entry(
    snapshot: &[u8; SLOT_BYTES],
    cursor: usize,
    used: usize,
) -> Result<(&[u8], NodeKind, &[u8], usize), StorageError> {
    if cursor + ENTRY_HEADER_BYTES > used {
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

fn checksum(bytes: &[u8]) -> u32 {
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

fn ata_read_sector(lba: u32, sector: &mut [u8; SECTOR_BYTES]) -> Result<(), StorageError> {
    select_sector(lba, ATA_COMMAND_READ)?;
    for index in 0..256 {
        let word = unsafe { arch::inw(ata_port(0)) }.to_le_bytes();
        sector[index * 2] = word[0];
        sector[index * 2 + 1] = word[1];
    }
    Ok(())
}

fn ata_write_sector(lba: u32, sector: &[u8; SECTOR_BYTES]) -> Result<(), StorageError> {
    select_sector(lba, ATA_COMMAND_WRITE)?;
    for chunk in sector.chunks_exact(2) {
        unsafe { arch::outw(ata_port(0), u16::from_le_bytes([chunk[0], chunk[1]])) };
    }
    wait_not_busy()
}

fn ata_flush() -> Result<(), StorageError> {
    wait_not_busy()?;
    unsafe { arch::outb(ata_port(7), ATA_COMMAND_FLUSH) };
    wait_not_busy()
}

fn select_sector(lba: u32, command: u8) -> Result<(), StorageError> {
    if lba >= 1 << 28 {
        return Err(StorageError::Device);
    }
    wait_not_busy()?;
    unsafe {
        arch::outb(ata_port(6), 0xe0 | ((lba >> 24) as u8 & 0x0f));
        arch::outb(ata_port(2), 1);
        arch::outb(ata_port(3), lba as u8);
        arch::outb(ata_port(4), (lba >> 8) as u8);
        arch::outb(ata_port(5), (lba >> 16) as u8);
        arch::outb(ata_port(7), command);
    }
    wait_drq()
}

fn wait_not_busy() -> Result<(), StorageError> {
    for _ in 0..ATA_POLL_LIMIT {
        let status = unsafe { arch::inb(ata_port(7)) };
        if status == 0 || status == u8::MAX || status & (ATA_STATUS_ERROR | ATA_STATUS_DF) != 0 {
            return Err(StorageError::Device);
        }
        if status & ATA_STATUS_BUSY == 0 {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(StorageError::Device)
}

fn wait_drq() -> Result<(), StorageError> {
    for _ in 0..ATA_POLL_LIMIT {
        let status = unsafe { arch::inb(ata_port(7)) };
        if status == 0 || status == u8::MAX || status & (ATA_STATUS_ERROR | ATA_STATUS_DF) != 0 {
            return Err(StorageError::Device);
        }
        if status & ATA_STATUS_BUSY == 0 && status & ATA_STATUS_DRQ != 0 {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(StorageError::Device)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_valid_generation_wins_and_torn_slot_is_ignored() {
        assert_eq!(
            newest_valid_slot(&[
                SlotSummary {
                    generation: Some(8),
                    blank: false,
                    readable: true,
                },
                SlotSummary {
                    generation: None,
                    blank: false,
                    readable: true,
                },
            ]),
            Some(0)
        );
    }
}
