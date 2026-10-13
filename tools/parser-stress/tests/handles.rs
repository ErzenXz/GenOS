use genos_parser_stress::handles::exercise;

fn command(action: u8, owner: u8, kind: u8, handle: u64, rights: u64) -> [u8; 20] {
    let mut record = [
        action, owner, kind, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    record[4..12].copy_from_slice(&handle.to_le_bytes());
    record[12..20].copy_from_slice(&rights.to_le_bytes());
    record
}

#[test]
fn exhaustion_duplicate_zero_wrong_type_and_revoke_preserve_live_authority() {
    let records = [
        command(0, 0, 0, 0, u64::MAX), // Zero is never an authority.
        command(0, 0, 0, 1, 0b11),
        command(0, 0, 1, 2, 1),
        command(0, 0, 3, 3, 0),
        command(0, 0, 6, 4, 1 << 63),
        command(0, 0, 5, 5, u64::MAX), // Full table.
        command(0, 0, 5, 1, u64::MAX), // Duplicate must not replace File.
        command(1, 0, 0, 1, 0b11),
        command(1, 0, 0, 1, 0b100), // Missing right.
        command(1, 0, 5, 1, 0),     // Wrong type, even with no required rights.
        command(2, 0, 5, 1, 0),     // Wrong-type revoke must not remove File.
        command(1, 0, 0, 1, 1),
        command(2, 0, 0, 1, 0),
        command(2, 0, 0, 1, 0), // Repeated revoke.
        command(1, 0, 0, 1, 0),
        command(0, 0, 5, 5, u64::MAX), // Reclaimed slot.
        command(1, 0, 5, 5, u64::MAX),
        command(3, 0, 0, 0, 0),
        command(1, 0, 5, 5, 0),
        command(0, 0, 0, 1, 1), // Re-registration has no imaginary generation.
    ];
    assert!(exercise(&records.concat()));
}

#[test]
fn wrong_owner_denial_and_independent_identical_values_survive_clear() {
    let records = [
        command(0, 0, 4, u64::MAX, 1),
        command(1, 1, 4, u64::MAX, 0),
        command(2, 1, 4, u64::MAX, 0),
        command(1, 0, 4, u64::MAX, 1),
        command(0, 1, 6, u64::MAX, 1 << 63),
        command(1, 0, 6, u64::MAX, 0),
        command(1, 1, 4, u64::MAX, 0),
        command(1, 1, 6, u64::MAX, 1 << 63),
        command(3, 0, 0, 0, 0),
        command(1, 1, 6, u64::MAX, 1 << 63),
        command(2, 1, 6, u64::MAX, 0),
        command(1, 1, 6, u64::MAX, 0),
    ];
    assert!(exercise(&records.concat()));
}

#[test]
fn every_kind_and_authority_bit_round_trip_through_both_owners() {
    for kind in 0..7 {
        for bit in 0..64 {
            let rights = 1u64 << bit;
            let records = [
                command(0, 0, kind, 0x41, rights),
                command(0, 1, kind, 0x41, !rights),
                command(1, 0, kind, 0x41, rights),
                command(1, 0, kind, 0x41, !rights),
                command(1, 1, kind, 0x41, rights),
                command(1, 1, kind, 0x41, !rights),
                command(2, 0, kind, 0x41, 0),
                command(1, 1, kind, 0x41, !rights),
                command(3, 1, 0, 0, 0),
            ];
            assert!(exercise(&records.concat()));
        }
    }
}

#[test]
fn empty_partial_and_large_inputs_remain_bounded() {
    for length in 0..20 {
        assert!(exercise(&[0xff; 20][..length]));
    }
    assert!(exercise(&vec![0xff; 65_536]));
}
