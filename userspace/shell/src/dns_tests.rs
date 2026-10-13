use super::*;

fn reply() -> std::vec::Vec<u8> {
    std::vec![
        0x47, 0x45, 0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0, 7, b'e', b'x', b'a', b'm', b'p', b'l',
        b'e', 3, b'c', b'o', b'm', 0, 0, 1, 0, 1, 0xc0, 12, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 93, 184,
        216, 34,
    ]
}

#[test]
fn compressed_reply_and_every_truncation() {
    let bytes = reply();
    assert_eq!(
        first_a(&bytes),
        Some(u32::from_be_bytes([93, 184, 216, 34]))
    );
    for end in 0..bytes.len() {
        assert_eq!(first_a(&bytes[..end]), None);
    }
}

#[test]
fn error_truncation_opcode_and_wrong_question_do_not_prove_resolution() {
    for (offset, value) in [(3, 0x83), (2, 0x83), (2, 0x89), (28, 2), (26, 28)] {
        let mut bytes = reply();
        bytes[offset] = value;
        assert_eq!(first_a(&bytes), None, "invalid response offset={offset}");
    }
}

#[test]
fn out_of_bounds_self_and_cyclic_compression_are_rejected() {
    for pointer in [0x3fff, 29, 30] {
        let mut bytes = reply();
        bytes[29] = 0xc0 | ((pointer >> 8) as u8);
        bytes[30] = pointer as u8;
        assert_eq!(first_a(&bytes), None, "invalid pointer={pointer}");
    }
    let mut bytes = reply();
    bytes[12..14].copy_from_slice(&[0xc0, 29]);
    assert_eq!(first_a(&bytes), None);
}

#[test]
fn expanded_names_have_a_bounded_wire_length() {
    let mut bytes = std::vec::Vec::new();
    for _ in 0..4 {
        bytes.push(63);
        bytes.extend_from_slice(&[b'a'; 63]);
    }
    bytes.push(0);
    assert_eq!(skip_dns_name(&bytes, 0), None);
    assert_eq!(skip_dns_name(&[1, b'a', 0], 0), Some(3));
    assert_eq!(skip_dns_name(&[0; 2], usize::MAX), None);
}
