//! Production GFS2 wire admission, mount atomicity and byte-preserving round trips.
use kernel::{
    storage_format as format,
    vfs::{NodeKind, RamVfs},
};

pub fn partition(bytes: &[u8]) -> bool {
    let Ok(partition) = format::parse_partition(bytes) else {
        return false;
    };
    assert_eq!(bytes.len(), format::SECTOR_BYTES);
    assert_eq!(&bytes[510..], &[0x55, 0xaa]);
    assert!(partition.start_lba > 0);
    assert!(partition.sectors > 2 * format::SLOT_SECTORS);
    assert!(partition.start_lba.checked_add(partition.sectors).is_some());
    assert!((0..4).any(|index| {
        let entry = &bytes[446 + 16 * index..][..16];
        u32::from_le_bytes(entry[8..12].try_into().unwrap()) == partition.start_lba
            && u32::from_le_bytes(entry[12..16].try_into().unwrap()) == partition.sectors
            && entry[4] == if partition.read_only { 0x7e } else { 0x7f }
    }));
    true
}

fn namespace(vfs: &RamVfs) -> Vec<(String, NodeKind, Vec<u8>)> {
    vfs.list_root()
        .map(|node| (node.path().into(), node.kind(), node.data().to_vec()))
        .collect()
}

fn check_snapshot(bytes: &[u8; format::SLOT_BYTES]) -> bool {
    let mut vfs = RamVfs::new();
    vfs.init_root();
    vfs.mkdir("/USER").unwrap();
    vfs.write("/SENTINEL", b"untouched").unwrap();
    let before = namespace(&vfs);
    let generation = format::validate_snapshot(bytes);
    let mounted = format::apply_snapshot(&mut vfs, bytes);
    if mounted.is_err() {
        assert_eq!(
            namespace(&vfs),
            before,
            "failed mount published partial entries"
        );
    } else {
        let generation = generation.expect("mounted snapshot must have passed admission");
        assert_eq!(vfs.read("/SENTINEL"), Ok(&b"untouched"[..]));
        let used = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
        let mut cursor = format::RECORD_HEADER_BYTES;
        for _ in 0..bytes[6] {
            let (path, kind, data, next) = format::decode_entry(bytes, cursor, used).unwrap();
            let path = std::str::from_utf8(path).unwrap();
            let node = vfs.find(path).unwrap();
            assert_eq!(node.kind(), kind);
            assert_eq!(node.data(), data);
            assert!(next > cursor && next <= used);
            cursor = next;
        }
        assert_eq!(cursor, used);
        // Encoding removes irrelevant padding, but must preserve admitted names,
        // types, contents and generation when committed and remounted.
        let mut encoded = [0; format::SLOT_BYTES];
        format::encode_snapshot(&vfs, generation, &mut encoded).unwrap();
        encoded[7] = format::RECORD_COMMITTED;
        let sum = format::checksum(&encoded);
        encoded[20..24].copy_from_slice(&sum.to_le_bytes());
        assert_eq!(format::validate_snapshot(&encoded), Ok(generation));
        let mut restored = RamVfs::new();
        restored.init_root();
        restored.mkdir("/USER").unwrap();
        restored.write("/SENTINEL", b"untouched").unwrap();
        format::apply_snapshot(&mut restored, &encoded).unwrap();
        assert_eq!(namespace(&restored), namespace(&vfs));
        // Reapplying a mounted namespace collides; rejection must be atomic.
        let before = namespace(&vfs);
        assert!(format::apply_snapshot(&mut vfs, bytes).is_err());
        assert_eq!(namespace(&vfs), before);
    }
    generation.is_ok()
}

pub fn snapshot(bytes: &[u8]) -> bool {
    let Ok(raw) = <&[u8; format::SLOT_BYTES]>::try_from(bytes) else {
        return false;
    };
    let accepted = check_snapshot(raw);
    // Also reach semantic admission after mutation instead of spending the
    // entire campaign on a checksum rejection. Raw admission is checked first.
    let mut repaired = *raw;
    let sum = format::checksum(&repaired);
    repaired[20..24].copy_from_slice(&sum.to_le_bytes());
    check_snapshot(&repaired);
    accepted
}
