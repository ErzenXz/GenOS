use genos_parser_stress::{exercise, TARGETS};

#[test]
fn every_target_accepts_its_seed_and_preserves_the_rejected_fixtures() {
    for (target, bytes, accepted) in [
        (
            "partition",
            &include_bytes!("../corpus/partition-valid.bin")[..],
            true,
        ),
        (
            "partition",
            &include_bytes!("../corpus/partition-overflow.bin")[..],
            false,
        ),
        (
            "partition",
            &include_bytes!("../corpus/partition-short.bin")[..],
            false,
        ),
        (
            "snapshot",
            &include_bytes!("../corpus/snapshot-valid.bin")[..],
            true,
        ),
        (
            "snapshot",
            &include_bytes!("../corpus/snapshot-checksum.bin")[..],
            false,
        ),
        (
            "snapshot",
            &include_bytes!("../corpus/snapshot-path.bin")[..],
            false,
        ),
        (
            "dhcp",
            &include_bytes!("../corpus/dhcp-valid.bin")[..],
            true,
        ),
        (
            "dhcp",
            &include_bytes!("../corpus/dhcp-truncated-option.bin")[..],
            false,
        ),
        ("dns", &include_bytes!("../corpus/dns-valid.bin")[..], true),
        (
            "dns",
            &include_bytes!("../corpus/dns-truncated.bin")[..],
            false,
        ),
        ("dns", &include_bytes!("../corpus/dns-error.bin")[..], false),
        (
            "dns",
            &include_bytes!("../corpus/dns-truncated-flag.bin")[..],
            false,
        ),
        (
            "dns",
            &include_bytes!("../corpus/dns-forward-pointer.bin")[..],
            false,
        ),
        (
            "dns",
            &include_bytes!("../corpus/dns-self-pointer.bin")[..],
            false,
        ),
        (
            "dns",
            &include_bytes!("../corpus/dns-wrong-question.bin")[..],
            false,
        ),
        ("arp", &include_bytes!("../corpus/arp-valid.bin")[..], true),
        (
            "icmp",
            &include_bytes!("../corpus/icmp-valid.bin")[..],
            true,
        ),
        (
            "syscalls",
            &include_bytes!("../corpus/syscalls-sequence.bin")[..],
            true,
        ),
        (
            "endpoints",
            &include_bytes!("../corpus/endpoints-sequence.bin")[..],
            true,
        ),
    ] {
        assert_eq!(exercise(target, bytes), accepted, "target={target}");
    }
}

#[test]
fn empty_partial_and_maximum_inputs_stay_bounded() {
    for target in TARGETS {
        for length in [
            0, 1, 3, 19, 20, 55, 56, 511, 512, 20479, 20480, 20481, 65536,
        ] {
            for byte in [0, 255] {
                exercise(target, &vec![byte; length]);
            }
        }
    }
}

#[test]
fn external_snapshot_decode_bounds_cannot_overflow_or_index_outside_the_slot() {
    use kernel::storage_format::{decode_entry, StorageError, SLOT_BYTES};
    let snapshot = [0; SLOT_BYTES];
    for (cursor, used) in [
        (usize::MAX, SLOT_BYTES),
        (SLOT_BYTES, usize::MAX),
        (SLOT_BYTES - 1, SLOT_BYTES),
        (64, 63),
    ] {
        assert_eq!(
            decode_entry(&snapshot, cursor, used),
            Err(StorageError::InvalidRecord)
        );
    }
}
